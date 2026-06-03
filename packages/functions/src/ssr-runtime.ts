// SSR handler — invoked by runtime.ts when the host sends a
// "render_route" message. Dynamically imports the page module, calls
// react-dom/server.renderToReadableStream, base64-encodes chunks back
// over the NDJSON pipe.
//
// The whole module is loaded lazily (handleRenderRoute is awaited from
// the dispatch arm) so projects without SSR routes pay nothing — no
// react-dom dependency requirement, no startup cost.

/**
 * The message payload the host sends. Matches RenderRouteMessage in
 * crates/functions/src/protocol.rs.
 */
export interface RenderRouteMessage {
  type: "render_route";
  call_id: string;
  /**
   * Project-relative module path (e.g. "app/hello/page"). The
   * adapter joins cwd + this + the right extension (.tsx → .ts).
   */
  component: string;
  /**
   * Project-relative module paths for the layout chain, walked
   * root → leaf. Each layout's default export wraps the next as
   * `children`. Absent / empty when no layouts apply.
   *
   * Example:
   *   layouts: ["app/layout", "app/blog/layout"]
   *   component: "app/blog/[slug]/page"
   *
   * Resolves to:
   *   <RootLayout>
   *     <BlogLayout>
   *       <Page {...props} />
   *     </BlogLayout>
   *   </RootLayout>
   */
  layouts?: string[];
  /** The matched route pattern (e.g. `/blog/:slug`). */
  route_path: string;
  /** The incoming URL path (e.g. `/blog/hello-world`). */
  url: string;
  /** Dynamic-segment matches keyed by name (e.g. `{slug: "hello-world"}`). */
  params: Record<string, string>;
  /** Parsed query string. */
  search_params: Record<string, string>;
  /** Lowercased header names → values. */
  headers: Record<string, string>;
  /** Parsed cookies. */
  cookies: Record<string, string>;
  /** Pylon auth context. */
  auth: {
    user_id: string | null;
    is_admin: boolean;
    tenant_id: string | null;
    roles: string[];
  };
}

type Send = (msg: Record<string, unknown>) => void;

/**
 * Control-flow signal a page or layout throws to short-circuit the
 * render: `response.redirect(url)` / `response.notFound()`. The adapter
 * catches it and turns it into a 3xx + Location or a 404 instead of a
 * normal 200 body. Extends Error so React's stream rejects cleanly when
 * it's thrown during the shell render. (Throw it OUTSIDE an error
 * boundary — an enclosing boundary would swallow the signal.)
 */
class PylonRouteControl extends Error {
  kind: "redirect" | "notFound";
  url?: string;
  redirectStatus?: number;
  constructor(kind: "redirect" | "notFound") {
    super(`__pylon_route_${kind}`);
    this.kind = kind;
  }
}

export interface SsrCookieOptions {
  path?: string;
  domain?: string;
  maxAge?: number;
  expires?: Date | string;
  /** Defaults to true (secure default). Pass false for a client-readable cookie. */
  httpOnly?: boolean;
  secure?: boolean;
  /** Defaults to "lax". */
  sameSite?: "strict" | "lax" | "none";
}

function serializeCookie(
  name: string,
  value: string,
  opts: SsrCookieOptions = {},
): string {
  let c = `${name}=${encodeURIComponent(value)}`;
  if (opts.maxAge != null) c += `; Max-Age=${Math.floor(opts.maxAge)}`;
  if (opts.expires) {
    c += `; Expires=${typeof opts.expires === "string" ? opts.expires : opts.expires.toUTCString()}`;
  }
  c += `; Path=${opts.path ?? "/"}`;
  if (opts.domain) c += `; Domain=${opts.domain}`;
  if (opts.httpOnly !== false) c += `; HttpOnly`;
  if (opts.secure) c += `; Secure`;
  const ss = opts.sameSite ?? "lax";
  c += `; SameSite=${ss[0].toUpperCase()}${ss.slice(1)}`;
  return c;
}

interface ResponseState {
  status: number;
  headers: Record<string, string>;
  cookies: string[];
}

/**
 * The per-render `response` controller handed to every page + layout in
 * props. Pylon already has a backend for data/mutations, so SSR's job is
 * just the response envelope: status, redirects, 404, and the occasional
 * Set-Cookie. Status/headers/cookies are read AFTER the shell renders, so
 * set them synchronously in the component body (before suspending).
 */
export interface SsrResponse {
  setStatus(code: number): void;
  setHeader(name: string, value: string): void;
  setCookie(name: string, value: string, opts?: SsrCookieOptions): void;
  redirect(url: string, status?: number): never;
  notFound(): never;
}

function makeResponseController(state: ResponseState): SsrResponse {
  return {
    setStatus(code) {
      state.status = code;
    },
    setHeader(name, value) {
      state.headers[name.toLowerCase()] = value;
    },
    setCookie(name, value, opts) {
      state.cookies.push(serializeCookie(name, value, opts));
    },
    redirect(url, status = 307): never {
      const e = new PylonRouteControl("redirect");
      e.url = url;
      e.redirectStatus = status;
      throw e;
    },
    notFound(): never {
      throw new PylonRouteControl("notFound");
    },
  };
}

/**
 * Merge page-set headers + cookies into the response_start header map.
 * Cookies are newline-joined under `set-cookie`; the host splits them
 * into one `Set-Cookie` header each (newline is forbidden inside a
 * cookie, so it can't be turned into header injection).
 */
function finalizeHeaders(
  state: ResponseState,
  extra?: Record<string, string>,
): Record<string, string> {
  const h: Record<string, string> = { ...state.headers, ...(extra ?? {}) };
  if (!h["content-type"]) h["content-type"] = "text/html; charset=utf-8";
  if (state.cookies.length > 0) h["set-cookie"] = state.cookies.join("\n");
  return h;
}

/**
 * Phase 1 SSR handler. Resolves the component, renders it via
 * react-dom/server.renderToReadableStream, pumps chunks back to the
 * host as base64-encoded NDJSON.
 *
 * Errors fall back to a type:"error" frame so the host can return a
 * 500 with the error body. Mid-stream errors (after the first chunk
 * has flushed) are uncatchable here — React's `onError` would have
 * to feed into a separate signal, deferred to Phase 1.5.
 */
export async function handleRenderRoute(
  msg: RenderRouteMessage,
  send: Send,
): Promise<void> {
  // Declared OUTSIDE the try so the catch can read page-set status/
  // cookies when turning a redirect()/notFound() throw into a response.
  const responseState: ResponseState = { status: 200, headers: {}, cookies: [] };
  const response = makeResponseController(responseState);
  try {
    // react + react-dom are USER deps. ssr-runtime.ts lives in
    // packages/functions/src/, but the user's react install is under
    // their project cwd. `import("react-dom/server")` in this file
    // would resolve against pylon's own node_modules (which doesn't
    // declare react), so we route through a Bun-resolveSync against
    // the user's cwd.
    const cwd = process.cwd();
    const resolveFromUser = (spec: string): string =>
      (Bun as any).resolveSync
        ? (Bun as any).resolveSync(spec, cwd)
        : spec;
    // `renderToReadableStream` is only exported from
    // `react-dom/server.browser` (WHATWG streams), not the plain
    // `react-dom/server` (which is Node-stream-style). Try browser
    // first, fall back to the default entry for environments that
    // re-route it (Next runs a custom dist).
    let reactDomServerImport: any;
    try {
      // @ts-ignore — user-dep, resolved at runtime
      reactDomServerImport = await import(
        /* @vite-ignore */ resolveFromUser("react-dom/server.browser")
      );
    } catch {
      // @ts-ignore — user-dep, resolved at runtime
      reactDomServerImport = await import(
        /* @vite-ignore */ resolveFromUser("react-dom/server")
      );
    }
    // @ts-ignore — user-dep, resolved at runtime
    const reactImport = await import(
      /* @vite-ignore */ resolveFromUser("react")
    );
    const React = reactImport.default ?? reactImport;
    const renderToReadableStream =
      reactDomServerImport.renderToReadableStream ??
      reactDomServerImport.default?.renderToReadableStream;
    if (typeof renderToReadableStream !== "function") {
      throw new Error(
        "react-dom/server.browser does not export renderToReadableStream — install react@>=18 + react-dom@>=18",
      );
    }

    // Resolve the page module. The component string is project-
    // relative without extension; try .tsx → .ts → .jsx → .js so
    // any of the common page-file shapes work. cwd was captured
    // above for the react resolver.
    const baseName = `${cwd}/${msg.component}`;
    let mod: any = null;
    let lastErr: unknown = null;
    for (const ext of [".tsx", ".ts", ".jsx", ".js"]) {
      try {
        mod = await import(`${baseName}${ext}`);
        break;
      } catch (e) {
        lastErr = e;
      }
    }
    if (!mod) {
      throw lastErr ?? new Error(`could not import component "${msg.component}"`);
    }
    const Component = mod.default ?? mod.Page ?? mod.page;
    if (typeof Component !== "function") {
      throw new Error(
        `component "${msg.component}" has no default export (or named export "Page")`,
      );
    }

    const props = {
      url: msg.url,
      params: msg.params,
      searchParams: msg.search_params,
      headers: msg.headers,
      cookies: msg.cookies,
      auth: msg.auth,
      // Response controller — a page/layout calls response.setStatus /
      // setHeader / setCookie / redirect / notFound to shape the reply.
      response,
    };

    // Resolve the layout chain. Each layout module exports a default
    // function that accepts the same props + `children`. Walk leaf →
    // root: start with the page component as `tree`, then for each
    // layout (innermost first) wrap it as the new tree. Result is
    // the outermost layout containing all nested layouts down to
    // the page.
    let tree: any = React.createElement(Component, props);
    const layouts = msg.layouts ?? [];
    if (layouts.length > 0) {
      // Resolve all layouts first so we fail fast on a missing one
      // BEFORE we start emitting headers / chunks.
      const layoutMods: any[] = [];
      for (const layoutPath of layouts) {
        const lBase = `${cwd}/${layoutPath}`;
        let lMod: any = null;
        for (const ext of [".tsx", ".ts", ".jsx", ".js"]) {
          try {
            lMod = await import(`${lBase}${ext}`);
            break;
          } catch {
            // try next extension
          }
        }
        if (!lMod) {
          throw new Error(
            `could not import layout "${layoutPath}" — checked .tsx / .ts / .jsx / .js`,
          );
        }
        const LayoutComp =
          lMod.default ?? lMod.Layout ?? lMod.layout;
        if (typeof LayoutComp !== "function") {
          throw new Error(
            `layout "${layoutPath}" has no default export (or named export "Layout")`,
          );
        }
        layoutMods.push(LayoutComp);
      }
      // Walk LEAF → ROOT (reverse iteration on the layouts array).
      // The innermost layout wraps the page first; each outer layout
      // wraps the result.
      for (let i = layoutMods.length - 1; i >= 0; i--) {
        tree = React.createElement(layoutMods[i], props, tree);
      }
    }
    const element = tree;
    const stream: ReadableStream<Uint8Array> = await renderToReadableStream(
      element,
      {
        onError(err: unknown) {
          // React captures render errors during the streaming render
          // and feeds them here. Phase 1 logs to stderr; Phase 1.5
          // sends a structured signal so the host can truncate the
          // body + emit a debug overlay.
          // eslint-disable-next-line no-console
          console.error("[ssr] renderToReadableStream onError:", err);
        },
      },
    );

    // Headers go out before the first chunk so the host can write the
    // response head.
    // The shell rendered without a redirect()/notFound() throw, so the
    // page's chosen status (default 200) + headers + cookies go out now,
    // before the first body byte.
    send({
      type: "response_start",
      call_id: msg.call_id,
      status: responseState.status,
      headers: finalizeHeaders(responseState),
    });

    // Pre-load the manifest BEFORE the React stream starts emitting
    // so we know which `<link rel="stylesheet">` and
    // `<link rel="modulepreload">` tags to inject into the HEAD.
    // We splice them in before `</head>` so the browser starts
    // fetching CSS + chunks concurrently with parsing the body —
    // no FOUC, no waterfall.
    let preloadManifestRoute:
      | { file: string; imports: string[]; css: string[] }
      | null = null;
    let preloadManifestErr: string | null = null;
    let preloadPublicPrefix = "/_pylon/build/";
    try {
      const { getManifest } = await import("./ssr-client-bundler");
      const manifest = await getManifest();
      preloadPublicPrefix = manifest.public_prefix || preloadPublicPrefix;
      preloadManifestRoute = manifest.routes[msg.component] ?? null;
      if (!preloadManifestRoute) {
        preloadManifestErr = `manifest has no entry for "${msg.component}"`;
      }
    } catch (e: any) {
      preloadManifestErr = e?.message || String(e);
    }

    // Build the head-injection blob — stylesheet first, then
    // modulepreloads. The entry script tag stays in the body-tail
    // (it needs the inline __PYLON_DATA__ to have been parsed first).
    let headBlob = "";
    if (preloadManifestRoute) {
      for (const css of preloadManifestRoute.css) {
        headBlob += `<link rel="stylesheet" href="${preloadPublicPrefix}${css}">`;
      }
      for (const chunk of preloadManifestRoute.imports) {
        headBlob += `<link rel="modulepreload" href="${preloadPublicPrefix}${chunk}">`;
      }
    }

    // Stream-rewrite: watch for `</head>` and inject `headBlob`
    // before it. `</head>` may straddle chunk boundaries so we
    // keep a small carry buffer (7 bytes — len("</head>")) at the
    // tail of each chunk.
    const reader = stream.getReader();
    let headInjected = headBlob.length === 0;
    let carry = ""; // utf8 tail from previous chunk for boundary detection
    const HEAD_CLOSE = "</head>";
    const sendChunk = (text: string) => {
      if (!text) return;
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: Buffer.from(text, "utf8").toString("base64"),
      });
    };
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      let text = Buffer.from(value).toString("utf8");
      if (!headInjected) {
        const combined = carry + text;
        const idx = combined.indexOf(HEAD_CLOSE);
        if (idx >= 0) {
          // Send everything up to the </head> position, then the
          // headBlob, then </head>, then the remainder.
          const before = combined.slice(0, idx);
          const after = combined.slice(idx + HEAD_CLOSE.length);
          // Drop the carry portion from `before` that we already
          // emitted as part of the previous chunk's send. But since
          // we DIDN'T emit `carry` previously (it was withheld), we
          // can send the full `before` here.
          sendChunk(before);
          sendChunk(headBlob);
          sendChunk(HEAD_CLOSE);
          if (after) sendChunk(after);
          headInjected = true;
          carry = "";
        } else {
          // No </head> yet — emit everything except the last
          // (HEAD_CLOSE.length - 1) bytes so a tag split across
          // chunk boundaries still gets caught next pass.
          const keep = HEAD_CLOSE.length - 1;
          if (combined.length > keep) {
            sendChunk(combined.slice(0, combined.length - keep));
            carry = combined.slice(combined.length - keep);
          } else {
            carry = combined;
          }
        }
      } else {
        // base64 in pure JS via Buffer (Bun ships it). For large
        // pages this is O(n) per chunk; fine for Phase 1.
        sendChunk(text);
      }
    }
    // Flush any residual carry (head close never seen — page
    // didn't have a </head>, which is fine for fragment renders).
    if (carry) sendChunk(carry);

    // Hydration tail. After React's stream EOFs we append the
    // hydration markers so the browser can hydrate:
    //   1. `__PYLON_DATA__` — JSON-typed script with the props the
    //      page was rendered with. The per-route bundle reads this
    //      to seed hydrateRoot.
    //   2. `<link rel="modulepreload">` for every transitive shared
    //      chunk (react, react-dom, client-runtime). These preload
    //      tags fire as soon as the parser sees them; the browser
    //      can start fetching while it's still parsing the body.
    //   3. `<script type="module" src="<route-entry>.js">` — the
    //      per-route entry. It imports the shared chunks, which
    //      were already in the cache from step 2.
    //
    // Per-route entry + chunk paths come from
    // `.pylon/client-build/manifest.json`, which the bundler writes
    // and `getManifest` parses with mtime-keyed caching. Falls back
    // to a no-hydration warning if the manifest can't be loaded
    // (rare — usually means the bundler crashed).
    const hydrationPayload = {
      component: msg.component,
      layouts: msg.layouts ?? [],
      props,
    };
    const json = JSON.stringify(hydrationPayload).replaceAll("<", "\\u003c");

    let tail = `<script id="__PYLON_DATA__" type="application/json">${json}</script>`;
    if (preloadManifestRoute) {
      // Per-route entry script comes last — it needs the inline
      // `__PYLON_DATA__` above to have been parsed before it runs.
      // CSS + modulepreload links were already injected into `<head>`
      // above so they could start fetching as early as possible.
      tail += `<script type="module" src="${preloadPublicPrefix}${preloadManifestRoute.file}"></script>`;
    } else {
      tail += `<script>console.warn(${JSON.stringify(`[pylon ssr] hydration disabled: ${preloadManifestErr}`)})</script>`;
    }
    send({
      type: "render_chunk",
      call_id: msg.call_id,
      data: Buffer.from(tail, "utf8").toString("base64"),
    });

    send({ type: "render_done", call_id: msg.call_id });
  } catch (err: any) {
    // A page/layout called response.redirect() or response.notFound()
    // during render → short-circuit to a 3xx + Location or a 404 instead
    // of a body. Page-set cookies/headers still ride along.
    if (err instanceof PylonRouteControl) {
      if (err.kind === "redirect") {
        send({
          type: "response_start",
          call_id: msg.call_id,
          status: err.redirectStatus ?? 307,
          headers: finalizeHeaders(responseState, { location: err.url ?? "/" }),
        });
        send({ type: "render_done", call_id: msg.call_id });
        return;
      }
      // notFound() → 404 with a minimal body (until not-found.tsx wiring).
      const body404 =
        '<!DOCTYPE html><html><head><meta charset="utf-8"><title>404 — Not Found</title></head><body><h1>404</h1><p>This page could not be found.</p></body></html>';
      send({
        type: "response_start",
        call_id: msg.call_id,
        status: 404,
        headers: finalizeHeaders(responseState),
      });
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: Buffer.from(body404, "utf8").toString("base64"),
      });
      send({ type: "render_done", call_id: msg.call_id });
      return;
    }
    // Real pre-first-chunk error → host returns 500.
    send({
      type: "error",
      call_id: msg.call_id,
      code: err?.code ?? "SSR_RENDER_FAILED",
      message: err?.message ?? String(err),
    });
  }
}
