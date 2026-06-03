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

/**
 * Reject CR / LF / NUL — the characters that turn a header or cookie
 * value into HTTP response splitting. The host re-checks at the wire
 * edge, but failing here gives the developer a clear error instead of a
 * silently-dropped header.
 */
function assertNoControlChars(s: string, label: string): void {
  if (/[\r\n\0]/.test(s)) {
    throw new Error(`pylon ssr: ${label} must not contain CR, LF, or NUL`);
  }
}

// RFC 6265 / RFC 7230 token — used for cookie + header NAMES.
const TOKEN_RE = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;

function serializeCookie(
  name: string,
  value: string,
  opts: SsrCookieOptions = {},
): string {
  // Cookie name must be a token — reject anything that could smuggle an
  // attribute or a newline through the name (the value is encoded below).
  if (!TOKEN_RE.test(name)) {
    throw new Error(`pylon ssr: invalid cookie name ${JSON.stringify(name)}`);
  }
  let c = `${name}=${encodeURIComponent(value)}`;
  if (opts.maxAge != null) c += `; Max-Age=${Math.floor(opts.maxAge)}`;
  if (opts.expires) {
    const exp =
      typeof opts.expires === "string"
        ? opts.expires
        : opts.expires.toUTCString();
    assertNoControlChars(exp, "cookie expires");
    c += `; Expires=${exp}`;
  }
  const path = opts.path ?? "/";
  assertNoControlChars(path, "cookie path");
  c += `; Path=${path}`;
  if (opts.domain) {
    assertNoControlChars(opts.domain, "cookie domain");
    c += `; Domain=${opts.domain}`;
  }
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
 * Set-Cookie.
 *
 * IMPORTANT — call these during the SYNCHRONOUS shell render (the
 * component body, before any `await` / Suspense boundary). The HTTP head
 * is committed when the shell is ready; status/headers/cookies set from a
 * suspended subtree that streams in later are lost, and a redirect()/
 * notFound() thrown below a Suspense boundary is caught by React's error
 * handling rather than turned into a 3xx/404 (same constraint as Next's
 * streaming SSR). Per render, not shared across requests.
 */
export interface SsrResponse {
  /** Set the HTTP status (100–599). Default 200. */
  setStatus(code: number): void;
  /** Set a response header (name must be a token; value CR/LF/NUL-free). */
  setHeader(name: string, value: string): void;
  /** Append a Set-Cookie. Defaults: HttpOnly + SameSite=Lax. */
  setCookie(name: string, value: string, opts?: SsrCookieOptions): void;
  /** Throw to send a 3xx (default 307) + Location, no body. Shell-render only. */
  redirect(url: string, status?: number): never;
  /** Throw to send a 404. Currently a fixed framework body (not-found.tsx not yet honored). Shell-render only. */
  notFound(): never;
}

function makeResponseController(state: ResponseState): SsrResponse {
  return {
    setStatus(code) {
      if (!Number.isInteger(code) || code < 100 || code > 599) {
        throw new Error(
          `pylon ssr: setStatus() expects an HTTP status 100–599, got ${code}`,
        );
      }
      state.status = code;
    },
    setHeader(name, value) {
      if (!TOKEN_RE.test(name)) {
        throw new Error(`pylon ssr: invalid header name ${JSON.stringify(name)}`);
      }
      assertNoControlChars(value, "header value");
      state.headers[name.toLowerCase()] = value;
    },
    setCookie(name, value, opts) {
      state.cookies.push(serializeCookie(name, value, opts));
    },
    redirect(url, status = 307): never {
      assertNoControlChars(url, "redirect url");
      if (!Number.isInteger(status) || status < 300 || status > 399) {
        throw new Error(`pylon ssr: redirect() status must be 3xx, got ${status}`);
      }
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
  if (state.cookies.length > 0) {
    // Preserve a set-cookie value set via setHeader() (rare) and join it
    // with setCookie() entries rather than clobbering it. The host splits
    // the newline-joined value into one Set-Cookie header each.
    const existing = h["set-cookie"];
    const all = existing ? [existing, ...state.cookies] : state.cookies;
    h["set-cookie"] = all.join("\n");
  }
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
/**
 * Page SEO metadata. A page exports `export const metadata = {...}`
 * (static) or `export async function generateMetadata(props)` (dynamic,
 * e.g. param-derived titles). Kept flat — no deep nesting beyond og/twitter.
 *
 * React 19 hoists the resulting <title>/<meta>/<link> into <head>. A page
 * `title` overrides a layout's static `<title>` (both render; the browser
 * uses the last, which is the page's). React does NOT dedupe arbitrary
 * `<meta>`, so set `description`/OG in EITHER the layout OR page metadata,
 * not both, to avoid duplicate tags.
 */
export interface SsrMetadata {
  title?: string;
  description?: string;
  keywords?: string | string[];
  canonical?: string;
  robots?: string;
  openGraph?: {
    title?: string;
    description?: string;
    image?: string;
    url?: string;
    type?: string;
  };
  twitter?: {
    card?: string;
    title?: string;
    description?: string;
    image?: string;
  };
}

/**
 * Build a React fragment of <title>/<meta>/<link> from a page's metadata.
 * React 19 auto-hoists these into <head> wherever they render, and the
 * host's </head> splice preserves them. React escapes all text/attrs, so
 * there's no manual XSS handling. Returns null when there's nothing to emit.
 */
function renderMetadata(React: any, m: SsrMetadata | undefined): any {
  if (!m) return null;
  const el = React.createElement;
  const kids: any[] = [];
  if (m.title != null) kids.push(el("title", { key: "t" }, m.title));
  if (m.description != null) {
    kids.push(el("meta", { key: "d", name: "description", content: m.description }));
  }
  const kw = Array.isArray(m.keywords) ? m.keywords.join(", ") : m.keywords;
  if (kw) kids.push(el("meta", { key: "kw", name: "keywords", content: kw }));
  if (m.robots) kids.push(el("meta", { key: "r", name: "robots", content: m.robots }));
  if (m.canonical) {
    kids.push(el("link", { key: "c", rel: "canonical", href: m.canonical }));
  }
  const og = m.openGraph;
  if (og) {
    if (og.title != null) kids.push(el("meta", { key: "ogt", property: "og:title", content: og.title }));
    if (og.description != null) kids.push(el("meta", { key: "ogd", property: "og:description", content: og.description }));
    if (og.image) kids.push(el("meta", { key: "ogi", property: "og:image", content: og.image }));
    if (og.url) kids.push(el("meta", { key: "ogu", property: "og:url", content: og.url }));
    if (og.type) kids.push(el("meta", { key: "ogy", property: "og:type", content: og.type }));
  }
  const tw = m.twitter;
  if (tw) {
    if (tw.card) kids.push(el("meta", { key: "twc", name: "twitter:card", content: tw.card }));
    if (tw.title != null) kids.push(el("meta", { key: "twt", name: "twitter:title", content: tw.title }));
    if (tw.description != null) kids.push(el("meta", { key: "twd", name: "twitter:description", content: tw.description }));
    if (tw.image) kids.push(el("meta", { key: "twi", name: "twitter:image", content: tw.image }));
  }
  return kids.length > 0 ? el(React.Fragment, null, ...kids) : null;
}

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

    // SEO metadata: static `export const metadata` or dynamic
    // `export async function generateMetadata(props)`. Awaited before the
    // first byte, so keep it to cheap derivations (params → title); heavy
    // data belongs in the page body behind <Suspense>.
    let metadata: SsrMetadata | undefined = mod.metadata;
    if (typeof mod.generateMetadata === "function") {
      metadata = await mod.generateMetadata(props);
    }
    const metaFragment = renderMetadata(React, metadata);

    // Resolve the layout chain. Each layout module exports a default
    // function that accepts the same props + `children`. Walk leaf →
    // root: start with the page component as `tree`, then for each
    // layout (innermost first) wrap it as the new tree. Result is
    // the outermost layout containing all nested layouts down to
    // the page. The metadata fragment is the FIRST child so React hoists
    // its <title>/<meta> into the <head> a layout renders.
    let tree: any = metaFragment
      ? React.createElement(
          React.Fragment,
          null,
          metaFragment,
          React.createElement(Component, props),
        )
      : React.createElement(Component, props);
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
