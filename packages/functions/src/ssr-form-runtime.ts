// route.ts form/method handler runtime (#276). Invoked by runtime.ts when the
// host sends a "handle_form" message. Imports the route.ts module, picks the
// handler by HTTP method, runs it with the parsed form + a read/write `db` +
// the SsrResponse controller, and replies over the SAME response_start /
// render_done protocol a render uses. A non-GET handler's common job is
// POST-redirect-GET: write something, then `response.redirect("/x?ok=1")`
// (303 by default here) so the no-JS browser follows with a GET.
//
// ANY handler may instead RETURN a raw response —
//   { body, contentType?, status?, headers? }
// — which is streamed verbatim, with no React render and no hydration tail.
// For `GET` that is the normal shape (dynamic RSS/Atom, XML, text, JSON: the
// GET analogue of `app/sitemap.ts`/`robots.ts` at an arbitrary path, where the
// default status is 200 rather than the form default of 303).
//
// The same return works on POST/PUT/PATCH/DELETE, because an endpoint that
// answers a machine has to answer with a body: a JSON API, a webhook receiver
// that must echo a challenge, a JSON-RPC endpoint such as MCP. Without it the
// only reply a non-GET route could make was a redirect, which is right for a
// browser form and wrong for everything else. Returning nothing keeps the
// POST-redirect-GET behavior, so existing handlers are untouched.
import {
  makeResponseController,
  PylonRouteControl,
  finalizeHeaders,
  importModule,
  isDevMode,
  type ResponseState,
} from "./ssr-runtime";
import { buildDbWriter } from "./runtime";

/** Matches HandleFormMessage in crates/functions/src/protocol.rs. */
export interface HandleFormMessage {
  type: "handle_form";
  call_id: string;
  component: string;
  route_path: string;
  method: string;
  url: string;
  params: Record<string, string>;
  search_params: Record<string, string>;
  form: Record<string, string | string[]>;
  /** The raw request body, as sent. Empty for GET. */
  body: string;
  headers: Record<string, string>;
  cookies: Record<string, string>;
  auth: {
    user_id: string | null;
    is_admin: boolean;
    tenant_id: string | null;
    roles: string[];
  };
}

type Send = (msg: Record<string, unknown>) => void;

/** Parsed form fields. Mirrors URLSearchParams get/getAll/has semantics. */
export interface FormFields {
  /** First value for `name`, or null. */
  get(name: string): string | null;
  /** All values for `name` (empty array if none). */
  getAll(name: string): string[];
  has(name: string): boolean;
  /** Raw map: name → value | values. */
  readonly fields: Record<string, string | string[]>;
}

function makeFormFields(raw: Record<string, string | string[]>): FormFields {
  return {
    get(name) {
      const v = raw[name];
      if (v == null) return null;
      return Array.isArray(v) ? (v.length > 0 ? v[0] : null) : v;
    },
    getAll(name) {
      const v = raw[name];
      if (v == null) return [];
      return Array.isArray(v) ? v : [v];
    },
    has(name) {
      return Object.prototype.hasOwnProperty.call(raw, name);
    },
    fields: raw,
  };
}

// GET is a raw handler (returns a body), the rest are form/mutation handlers
// (return void + use the response controller). All are dispatched the same way;
// the list drives the 405 `Allow` header advertising which a route.ts exports.
const HANDLER_METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"] as const;
const b64 = (s: string) => Buffer.from(s, "utf8").toString("base64");
// Share ONE dev-mode parse with ssr-runtime (case-insensitive "1"/"true") so
// the form path and the render path never disagree on whether to leak stacks.
const isDev = isDevMode;

export async function handleForm(
  msg: HandleFormMessage,
  send: Send,
): Promise<void> {
  const cwd = process.cwd();
  // Default 303 See Other — the correct POST-redirect-GET status for a form
  // (307 would re-issue the POST to the redirect target).
  const responseState: ResponseState = { status: 303, headers: {}, cookies: [] };
  const response = makeResponseController(responseState, 303);
  const req = {
    form: makeFormFields(msg.form ?? {}),
    // The exact bytes. `form` is only populated for urlencoded bodies, so a
    // JSON API / JSON-RPC / signature-verifying webhook handler reads this.
    body: msg.body ?? "",
    params: msg.params,
    searchParams: msg.search_params,
    auth: msg.auth,
    cookies: msg.cookies,
    headers: msg.headers,
    // Read+write DB handle (mutation-shaped; the host answers writes against a
    // broadcast-capable store). serverData (read-only) isn't given — a handler
    // writes via `db` and the developer enforces trust with `req.auth`.
    db: buildDbWriter(msg.call_id),
    response,
  };

  let mod: any;
  try {
    mod = await importModule(cwd, msg.component);
  } catch (e: any) {
    send({
      type: "error",
      call_id: msg.call_id,
      code: "SSR_FORM_IMPORT_FAILED",
      message: isDev() && e?.stack ? String(e.stack) : e?.message ?? String(e),
    });
    return;
  }

  const method = (msg.method || "POST").toUpperCase();
  const handler = mod[method];
  if (typeof handler !== "function") {
    // 405 — advertise the methods this route.ts actually exports.
    const allow = HANDLER_METHODS.filter(
      (m) => typeof mod[m] === "function",
    ).join(", ");
    send({
      type: "response_start",
      call_id: msg.call_id,
      status: 405,
      headers: {
        "content-type": "text/plain; charset=utf-8",
        ...(allow ? { allow } : {}),
      },
    });
    send({
      type: "render_chunk",
      call_id: msg.call_id,
      data: b64(`Method ${method} not allowed`),
    });
    send({ type: "render_done", call_id: msg.call_id });
    return;
  }

  // A raw GET handler returns its body; the default status is 200 (not the
  // POST-redirect-GET 303 that form handlers use). The handler can still
  // override via the returned object or `response.setStatus`.
  if (method === "GET") {
    responseState.status = 200;
  }

  try {
    const out = await handler(req);
    // A returned object is a RAW response, whatever the method. GET always
    // takes this path (its return value IS the reply, even when empty); the
    // other methods take it only when the handler actually returned one, so a
    // form handler that returns void still gets POST-redirect-GET.
    const returnedRaw =
      out != null &&
      typeof out === "object" &&
      ("body" in out || "contentType" in out || "status" in out || "headers" in out);
    if (method === "GET" || returnedRaw) {
      // Stream `out.body` with the handler's content-type/status/headers
      // (merged with anything set via the response controller). No React, no
      // hydration tail — verbatim bytes, like sitemap/robots.
      const raw = (out ?? {}) as {
        body?: unknown;
        contentType?: string;
        status?: number;
        headers?: Record<string, string>;
      };
      const extra: Record<string, string> = {};
      extra["content-type"] =
        raw.contentType ??
        responseState.headers["content-type"] ??
        "text/plain; charset=utf-8";
      for (const [k, v] of Object.entries(raw.headers ?? {})) {
        extra[k.toLowerCase()] = String(v);
      }
      // A non-GET handler's response state still defaults to 303 (the form
      // default). A raw return that names no status means 200 — a JSON reply
      // with an accidental 303 and no Location is a broken response.
      const rawStatus =
        raw.status ??
        (method === "GET" || responseState.status !== 303 ? responseState.status : 200);
      send({
        type: "response_start",
        call_id: msg.call_id,
        status: rawStatus,
        // Pass rawStatus so the open-redirect guard sees the handler's own
        // status (a raw GET's redirect status lives on `raw.status`, not
        // `responseState.status`).
        headers: finalizeHeaders(responseState, extra, undefined, rawStatus),
      });
      if (raw.body != null) {
        const bodyStr =
          typeof raw.body === "string" ? raw.body : String(raw.body);
        send({ type: "render_chunk", call_id: msg.call_id, data: b64(bodyStr) });
      }
      send({ type: "render_done", call_id: msg.call_id });
      return;
    }
    // No redirect()/notFound() thrown → commit the handler's response: its
    // status (default 303) + headers + cookies. A 303 with no explicit
    // Location redirects back to the route path (POST-redirect-GET).
    const extra: Record<string, string> = {};
    if (responseState.status === 303 && !responseState.headers["location"]) {
      extra["location"] = msg.route_path || "/";
    }
    send({
      type: "response_start",
      call_id: msg.call_id,
      status: responseState.status,
      headers: finalizeHeaders(responseState, extra),
    });
    send({ type: "render_done", call_id: msg.call_id });
  } catch (err: any) {
    // response.redirect()/notFound() throw PylonRouteControl — turn into a
    // 3xx + Location / 404, carrying any cookies the handler set first.
    if (err instanceof PylonRouteControl) {
      if (err.kind === "redirect") {
        send({
          type: "response_start",
          call_id: msg.call_id,
          status: err.redirectStatus ?? 303,
          headers: finalizeHeaders(responseState, { location: err.url ?? "/" }),
        });
        send({ type: "render_done", call_id: msg.call_id });
        return;
      }
      send({
        type: "response_start",
        call_id: msg.call_id,
        status: 404,
        headers: finalizeHeaders(responseState),
      });
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: b64("Not found"),
      });
      send({ type: "render_done", call_id: msg.call_id });
      return;
    }
    send({
      type: "error",
      call_id: msg.call_id,
      code: err?.code ?? "SSR_FORM_FAILED",
      message:
        isDev() && err?.stack ? String(err.stack) : err?.message ?? String(err),
    });
  }
}
