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
    // react + react-dom are USER deps (resolved against the user's
    // node_modules/), not Pylon's. `@ts-ignore` on the imports so
    // this file typechecks without bundling react into our own
    // package.json. The imports fire lazily inside the handler so
    // projects without SSR routes don't even attempt resolution.
    // @ts-ignore — user-dep, resolved at runtime
    const reactDomServerImport = await import("react-dom/server");
    // @ts-ignore — user-dep, resolved at runtime
    const reactImport = await import("react");
    const React = reactImport.default ?? reactImport;
    const renderToReadableStream =
      (reactDomServerImport as any).renderToReadableStream ??
      (reactDomServerImport.default as any)?.renderToReadableStream;
    if (typeof renderToReadableStream !== "function") {
      throw new Error(
        "react-dom/server does not export renderToReadableStream — install react@>=18 + react-dom@>=18",
      );
    }

    // Resolve the page module. The component string is project-
    // relative without extension; try .tsx → .ts → .jsx → .js so
    // any of the common page-file shapes work.
    const cwd = process.cwd();
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

    const element = React.createElement(Component, props);
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
