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
    send({
      type: "response_start",
      call_id: msg.call_id,
      status: 200,
      headers: { "content-type": "text/html; charset=utf-8" },
    });

    const reader = stream.getReader();
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      // base64 in pure JS via Buffer (Bun ships it). For large
      // pages this is O(n) per chunk; fine for Phase 1.
      const b64 = Buffer.from(value).toString("base64");
      send({
        type: "render_chunk",
        call_id: msg.call_id,
        data: b64,
      });
    }

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

    let manifestRoute: { file: string; imports: string[]; css: string[] } | null = null;
    let manifestErr: string | null = null;
    let publicPrefix = "/_pylon/build/";
    try {
      const { getManifest } = await import("./ssr-client-bundler");
      const manifest = await getManifest();
      publicPrefix = manifest.public_prefix || publicPrefix;
      manifestRoute = manifest.routes[msg.component] ?? null;
      if (!manifestRoute) {
        manifestErr = `manifest has no entry for "${msg.component}"`;
      }
    } catch (e: any) {
      manifestErr = e?.message || String(e);
    }

    let tail = `<script id="__PYLON_DATA__" type="application/json">${json}</script>`;
    if (manifestRoute) {
      // Preload shared chunks (react + runtime) first so they're
      // already in cache by the time the entry script's import
      // resolution starts. Order: preloads first, then entry.
      for (const chunk of manifestRoute.imports) {
        tail += `<link rel="modulepreload" href="${publicPrefix}${chunk}">`;
      }
      for (const css of manifestRoute.css) {
        tail += `<link rel="stylesheet" href="${publicPrefix}${css}">`;
      }
      tail += `<script type="module" src="${publicPrefix}${manifestRoute.file}"></script>`;
    } else {
      tail += `<script>console.warn(${JSON.stringify(`[pylon ssr] hydration disabled: ${manifestErr}`)})</script>`;
    }
    send({
      type: "render_chunk",
      call_id: msg.call_id,
      data: Buffer.from(tail, "utf8").toString("base64"),
    });

    send({ type: "render_done", call_id: msg.call_id });
  } catch (err: any) {
    // Pre-first-chunk error → host returns 500.
    send({
      type: "error",
      call_id: msg.call_id,
      code: err?.code ?? "SSR_RENDER_FAILED",
      message: err?.message ?? String(err),
    });
  }
}
