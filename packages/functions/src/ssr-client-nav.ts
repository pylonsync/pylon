// Instant client navigation: which routes the client runtime renders with no
// page fetch.
//
// A client navigation normally fetches the destination's SSR payload for its
// props and serverData, then renders. A route that reads neither does not
// need that round trip: its data comes from the local replica (db.useQuery),
// and the props it may read are the identity-free ones the last page had.
// Such a route opts in one of two ways:
//
//   - its page declares `export const navigation = "client"`, or
//   - its URL is one of `buildManifest({ offline: { routes } })`, which
//     already follow the same render rule (see ssr-offline.ts).
//
// The client runtime imports this module as `./client-nav` (the bundler
// stages this file next to it, like ./route-match), and the bundler uses
// `declaresClientNavigation` to mark pages in the bundle manifest. No imports:
// the staged copy must stand alone.

/** A route match, as `matchRoute` in ssr-route-match.ts returns it. */
export interface ClientRouteMatch {
  component: string;
  params: Record<string, string | string[]>;
}

/** The bundle manifest fields this module reads. */
export interface ClientNavManifest {
  routes: Record<string, { path?: string; navigation?: string }>;
  offline?: { routes?: string[] };
}

type MatchRoute = (manifest: any, pathname: string) => ClientRouteMatch | null;

/**
 * Whether page source declares `export const navigation = "client"` (single,
 * double or backtick quotes, with an optional type annotation or `as const`).
 * A static scan: the bundler reads the page file without running it.
 */
export function declaresClientNavigation(source: string): boolean {
  return /export\s+const\s+navigation\s*(?::[^=]+)?=\s*(["'`])client\1/.test(source);
}

/** Whether `pathname` is one of the app's offline routes. */
export function isOfflinePath(
  manifest: ClientNavManifest | null,
  pathname: string,
  matchRoute: MatchRoute,
): boolean {
  const patterns = manifest?.offline?.routes;
  if (!patterns || patterns.length === 0) return false;
  const routes: Record<string, { path: string }> = {};
  for (const pattern of patterns) routes[pattern] = { path: pattern };
  return matchRoute({ routes }, pathname) !== null;
}

/**
 * The page to render from the client for `pathname`, or null when the
 * navigation must fetch the page (no match, or the page did not opt in).
 */
export function clientRouteFor(
  manifest: ClientNavManifest | null,
  pathname: string,
  matchRoute: MatchRoute,
): ClientRouteMatch | null {
  if (!manifest) return null;
  const matched = matchRoute(manifest, pathname);
  if (!matched) return null;
  if (manifest.routes[matched.component]?.navigation === "client") return matched;
  if (isOfflinePath(manifest, pathname, matchRoute)) return matched;
  return null;
}
