// SSR client-runtime not-found / error boundary.
//
// Extracted from `CLIENT_RUNTIME_SOURCE` in ssr-client-bundler.ts so its
// RENDER behavior is testable with a real React renderer — the inline copy
// was a string literal, so nothing could render it, and two real regressions
// (a client-thrown notFound() blanking the page, then rendering with the
// wrong props and redirecting to /login) shipped because unit/build tests
// couldn't reach render-time behavior.
//
// The bundler writes this module's SOURCE next to the generated client
// runtime (as `.pylon/client-boundary.ts`) and the runtime imports
// `createPylonBoundary` from "./client-boundary", wiring its own internals
// (loadManifest / loadRouteEntry / navigate / buildTree / the active page
// props) in as deps. ssr-client-boundary.test.ts imports the same factory and
// drives it directly. ONE source of truth, no inline drift.
//
// Browser-safe by construction: imports ONLY "react" and touches NO DOM
// globals — every browser capability (the manifest, navigation, the current
// href) arrives through injected deps. So it bundles cleanly into the client
// chunk AND typechecks without a DOM lib.

import { Component, createElement, useEffect, useState } from "react";

/** A per-directory module resolved by walking up from a route: the two error
 *  boundaries, plus `loading` (the pending-navigation skeleton). */
export type BoundaryFile = "not-found" | "error" | "loading";

/**
 * Resolve the nearest `<dir>/<fileName>` module for a route by walking its
 * component path up to the app root, returning the first key that exists.
 * Nearest ancestor wins — the same model the server's `findBoundary` uses, but
 * driven off client-side keys so the runtime needs no extra server round-trip.
 *
 * `component` is a cwd-relative path with "/" separators and no extension
 * (e.g. "web/app/dashboard/orgs/[slug]/page"). `keys` is the set to resolve
 * against: `Object.keys(manifest.routes)` for not-found/error, which ship as
 * route entries, or the loading registry's keys for `loading`, which ships in
 * the shared chunk instead.
 */
export function nearestBoundaryComponent(
  component: string,
  fileName: BoundaryFile,
  routeKeys: Iterable<string>,
): string | null {
  const keys = routeKeys instanceof Set ? routeKeys : new Set(routeKeys);
  let dir = String(component);
  dir = dir.includes("/") ? dir.slice(0, dir.lastIndexOf("/")) : "";
  while (dir) {
    const key = `${dir}/${fileName}`;
    if (keys.has(key)) return key;
    const slash = dir.lastIndexOf("/");
    dir = slash >= 0 ? dir.slice(0, slash) : "";
  }
  return null;
}

/** Runtime internals the boundary needs, injected so the module stays
 *  browser-safe AND unit-testable. */
export interface BoundaryDeps {
  /** Resolve the client build manifest (`{ routes: {...} }`). */
  loadManifest: () => Promise<{ routes?: Record<string, unknown> } | null>;
  /** Dynamically load a route entry → its Page + Layout chain. */
  loadRouteEntry: (component: string) => Promise<{ Page: any; Layouts: any[] }>;
  /** Client-side navigation — used by an error boundary's reset(). */
  navigate: (href: string, opts?: { replace?: boolean }) => void;
  /** Wrap a Page in its Layout chain with props (the runtime's buildTree). */
  buildTree: (Page: any, Layouts: any[], props: any) => any;
  /** The props the active page was rendered with (auth, serverData, …). */
  getPageProps: () => any;
  /** Current same-origin href, for an error boundary's reset() re-navigation. */
  getResetHref: () => string;
}

/**
 * Build the client error / not-found boundary against the runtime's internals.
 * Returns `withBoundary(tree, component, navEpoch)` — the transparent root
 * wrapper the runtime puts around every page — plus `PylonBoundary` for tests.
 *
 * Behavior: a `notFound()` (digest "PYLON_NOT_FOUND") or any error thrown
 * during a descendant's render is caught; the nearest not-found.tsx / error.tsx
 * for the active route is resolved from the manifest, loaded, and rendered in
 * its own layout chain WITH the page's props (so an auth-guarding layout sees
 * the real auth and doesn't redirect away). Resets on navigation via a changing
 * `navEpoch` prop — not a key — so layouts keep their state across nav.
 */
export function createPylonBoundary(deps: BoundaryDeps) {
  const {
    loadManifest,
    loadRouteEntry,
    navigate,
    buildTree,
    getPageProps,
    getResetHref,
  } = deps;

  async function resolveBoundaryComponent(
    component: string,
    fileName: "not-found" | "error",
  ): Promise<string | null> {
    const manifest = await loadManifest();
    if (!manifest || !manifest.routes || !component) return null;
    return nearestBoundaryComponent(
      component,
      fileName,
      Object.keys(manifest.routes),
    );
  }

  // Last-resort body when the app ships no not-found.tsx / error.tsx anywhere.
  // Renders a full <html> document so it can replace the document root cleanly
  // (the normal path renders the app's boundary wrapped in the root layout,
  // which also owns <html>).
  function DefaultBoundary(props: any) {
    const isNF = props.kind === "not-found";
    return createElement(
      "html",
      { lang: "en" },
      createElement(
        "head",
        null,
        createElement("meta", { charSet: "utf-8" }),
        createElement(
          "title",
          null,
          isNF ? "404 — Not found" : "Something went wrong",
        ),
      ),
      createElement(
        "body",
        {
          style: {
            margin: 0,
            minHeight: "100vh",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontFamily: "system-ui, -apple-system, sans-serif",
          },
        },
        createElement(
          "div",
          { style: { textAlign: "center", padding: "2rem" } },
          createElement(
            "h1",
            { style: { fontSize: "1.375rem", margin: "0 0 0.5rem" } },
            isNF ? "404 — Not found" : "Something went wrong",
          ),
          createElement(
            "p",
            { style: { color: "#666", margin: "0 0 1.25rem" } },
            isNF
              ? "This page doesn't exist or was moved."
              : "An unexpected error occurred.",
          ),
          createElement(
            "a",
            { href: "/", style: { color: "#0969da", textDecoration: "none" } },
            "← Go home",
          ),
        ),
      ),
    );
  }

  // Lazily resolves + renders the boundary component in its own layout chain.
  // Renders nothing for the brief moment before the (usually already-cached)
  // entry loads, then the styled boundary — far better than a permanent blank.
  function BoundaryView(props: any) {
    const { component, kind, error, reset } = props;
    const [resolved, setResolved] = useState<any>(null);
    useEffect(() => {
      let cancelled = false;
      const fileName = kind === "not-found" ? "not-found" : "error";
      (async () => {
        const comp = await resolveBoundaryComponent(component, fileName);
        if (cancelled) return;
        if (!comp) {
          setResolved({ missing: true });
          return;
        }
        try {
          const entry = await loadRouteEntry(comp);
          if (!cancelled) setResolved({ entry });
        } catch {
          if (!cancelled) setResolved({ missing: true });
        }
      })();
      return () => {
        cancelled = true;
      };
    }, [component, kind]);
    if (!resolved) return null;
    if (resolved.missing) return createElement(DefaultBoundary, { kind });
    // Reuse the page's props (auth, serverData, params, url, …) so the
    // boundary's layout chain renders with the SAME context the page had — an
    // auth-guarding layout that reads props.auth must not see undefined and
    // redirect away. error.tsx gets the SAFE error projection (message + digest
    // only, never a raw Error/stack — matches the server boundary's #270).
    const bProps: any = { ...getPageProps(), reset };
    if (kind === "error" && error) {
      bProps.error = {
        message: String((error && error.message) || error),
        digest: error && error.digest,
      };
    }
    return buildTree(resolved.entry.Page, resolved.entry.Layouts, bProps);
  }

  class PylonBoundary extends Component<any, { err: any }> {
    constructor(p: any) {
      super(p);
      this.state = { err: null };
    }
    static getDerivedStateFromError(err: any) {
      return { err };
    }
    componentDidUpdate(prev: any) {
      // A navigation bumps navEpoch — clear a prior error so the freshly
      // rendered children (the new page) show instead of the stale boundary.
      if (prev.navEpoch !== this.props.navEpoch && this.state.err) {
        this.setState({ err: null });
      }
    }
    componentDidCatch(err: any) {
      // notFound() is expected control flow, not a crash — keep it quiet.
      // Genuine errors stay loud.
      if (!(err && err.digest === "PYLON_NOT_FOUND")) {
        // eslint-disable-next-line no-console
        console.error("[pylon ssr] render error caught by boundary:", err);
      }
    }
    render() {
      const err = this.state.err;
      if (!err) return this.props.children;
      const kind = err.digest === "PYLON_NOT_FOUND" ? "not-found" : "error";
      const self = this;
      return createElement(BoundaryView, {
        component: this.props.component,
        kind,
        error: err,
        reset() {
          self.setState({ err: null });
          navigate(getResetHref(), { replace: true });
        },
      });
    }
  }

  // Wrap a page tree in the root boundary. navEpoch (a prop, not a key) lets the
  // boundary clear its error on navigation without remounting the layouts.
  function withBoundary(tree: any, component: string, navEpoch: number) {
    return createElement(PylonBoundary, { navEpoch, component }, tree);
  }

  return { withBoundary, PylonBoundary };
}
