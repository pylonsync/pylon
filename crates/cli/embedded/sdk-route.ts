// Route definitions. This module has no runtime dependency on Node
// builtins, so client bundlers (Metro for React Native, Vite, Next.js)
// can import `defineRoute` through `@pylonsync/sdk/route` without
// pulling in the filesystem-backed discovery helpers from the package
// root. `@pylonsync/react` re-exports from here for that reason.

export type RouteMode = "static" | "server" | "live" | "ssr";

export type AuthMode = "public" | "user";

export interface RouteDefinition {
  path: string;
  mode: RouteMode;
  query?: string;
  auth?: AuthMode;
  /**
   * Project-relative module path (e.g. `app/hello/page`) for SSR
   * routes. Required when `mode === "ssr"`. Discovered automatically
   * by `discoverAppRoutes()`; only specify manually for one-off
   * SSR routes outside the `app/` tree.
   */
  component?: string;
  /**
   * Layout module path chain (root→leaf). Each layout wraps the next
   * as `children`. Only relevant for `mode === "ssr"`.
   */
  layouts?: string[];
  /**
   * Route kind. Omitted (or `"page"`) is a normal navigable page.
   * `"not-found"` / `"error"` are SSR boundary modules discovered from
   * `app/.../not-found.tsx` and `app/.../error.tsx`. Boundary routes are
   * NOT matched as navigable URLs — the host renders `not-found` for
   * unmatched URLs (HTTP 404) and `error` on render failure (HTTP 500).
   * `path` records the segment prefix the boundary covers (`/` for root).
   * `"route"` is a form/method handler (`app/.../route.ts` exporting
   * POST/PUT/PATCH/DELETE) — matched on its `path` for non-GET requests only,
   * never rendered as a page.
   */
  kind?:
    | "page"
    | "not-found"
    | "error"
    | "route"
    | "sitemap"
    | "robots"
    | "llms"
    | "og-image";
}

export function defineRoute(route: RouteDefinition): RouteDefinition {
  return route;
}
