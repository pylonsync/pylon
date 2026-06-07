// route.ts form/method handler runtime (#276). Invoked by runtime.ts when the
// host sends a "handle_form" message. Imports the route.ts module, picks the
// handler by HTTP method, runs it with the parsed form + a read/write `db` +
// the SsrResponse controller, and replies over the SAME response_start /
// render_done protocol a render uses. A handler's common job is
// POST-redirect-GET: write something, then `response.redirect("/x?ok=1")`
// (303 by default here) so the no-JS browser follows with a GET.
import {
  makeResponseController,
  PylonRouteControl,
  finalizeHeaders,
  importModule,
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

const HANDLER_METHODS = ["POST", "PUT", "PATCH", "DELETE"] as const;
const b64 = (s: string) => Buffer.from(s, "utf8").toString("base64");
const isDev = () =>
  process.env.PYLON_DEV_MODE === "1" || process.env.PYLON_DEV_MODE === "true";

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

  try {
    await handler(req);
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
