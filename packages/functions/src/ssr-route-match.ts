// Client-side route matcher for optimistic navigation.
//
// The SSR server owns the authoritative path→component mapping, but for
// optimistic navigation (painting a destination's Suspense fallback with a
// `<Link seed>` BEFORE the SSR fetch resolves) the client must resolve the
// destination route from the clicked href on its own. The build manifest ships
// each page route's `path` pattern (e.g. `/p/[slug]`) for exactly this — this
// module is the pure matcher over those patterns.
//
// Kept in its own module (not inlined in CLIENT_RUNTIME_SOURCE) so it has one
// source of truth and is unit-tested directly. The client bundler stages a copy
// next to the generated runtime, which imports it as `./route-match`.

export interface RouteMatch {
  /** Route component path — the manifest key + __PYLON_DATA__.component value. */
  component: string;
  /** Decoded dynamic params captured from the path (e.g. `{ slug: "shoe-x" }`). */
  params: Record<string, string>;
}

/** The slice of the build manifest this module needs. */
export interface MatchableManifest {
  routes: Record<
    string,
    {
      path?: string;
      /** The route's own entry chunk (outdir-relative). */
      file?: string;
      /** Shared chunks the browser needs before that entry runs. */
      imports?: string[];
    }
  >;
}

function splitPath(p: string): string[] {
  const trimmed = p.replace(/\/+$/, "");
  return trimmed === "" ? [] : trimmed.slice(1).split("/");
}

interface SegMatch {
  params: Record<string, string>;
  /** Count of `[param]` segments — lower is more specific. */
  dynamic: number;
  /** Count of `[...rest]` catch-alls — dominates specificity. */
  catchAll: number;
}

// Match a route pattern's segments against a concrete path's segments. Handles
// static segments, `[param]` (single segment), and `[...param]` (catch-all,
// consumes the remainder). Returns null on any mismatch.
function matchSegments(
  patSegs: string[],
  targetSegs: string[],
): SegMatch | null {
  const params: Record<string, string> = {};
  let dynamic = 0;
  for (let i = 0; i < patSegs.length; i++) {
    const seg = patSegs[i];
    if (seg.startsWith("[...") && seg.endsWith("]")) {
      const name = seg.slice(4, -1);
      params[name] = targetSegs
        .slice(i)
        .map(safeDecode)
        .join("/");
      return { params, dynamic, catchAll: 1 };
    }
    if (i >= targetSegs.length) return null;
    if (seg.startsWith("[") && seg.endsWith("]")) {
      params[seg.slice(1, -1)] = safeDecode(targetSegs[i]);
      dynamic++;
    } else if (seg !== targetSegs[i]) {
      return null;
    }
  }
  // No catch-all consumed the tail, so the lengths must line up exactly.
  if (targetSegs.length !== patSegs.length) return null;
  return { params, dynamic, catchAll: 0 };
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/**
 * Resolve a concrete pathname to the route that would render it, plus its
 * decoded dynamic params. Returns null when no page route matches (the caller
 * then falls back to a normal server round-trip). When several patterns match,
 * the most specific wins: fewer catch-alls first, then fewer dynamic segments —
 * so `/orders/new` beats `/orders/[id]` beats `/[...all]`.
 */
export function matchRoute(
  manifest: MatchableManifest | null | undefined,
  pathname: string,
): RouteMatch | null {
  if (!manifest || !manifest.routes) return null;
  const targetSegs = splitPath(pathname);
  let best: { match: RouteMatch; score: number } | null = null;
  for (const component of Object.keys(manifest.routes)) {
    const route = manifest.routes[component];
    if (!route || typeof route.path !== "string") continue;
    const m = matchSegments(splitPath(route.path), targetSegs);
    if (!m) continue;
    const score = m.catchAll * 1000 + m.dynamic;
    if (best === null || score < best.score) {
      best = { match: { component, params: m.params }, score };
    }
  }
  return best ? best.match : null;
}

/** What `<Link prefetch>` should warm for a destination href. */
export interface PrefetchTargets {
  /** The destination route's own entry chunk, or "" when no page route matches. */
  file: string;
  /** Chunks to warm alongside it. */
  imports: string[];
}

/**
 * The chunks a click on `pathname` will need before anything can render: the
 * destination route's entry, plus the chunks (React, the client runtime,
 * common layouts) it pulls in.
 *
 * Warming the page payload alone leaves the entry chunk to be fetched after
 * the click, and the route cannot render until it lands — so the prefetch
 * covers only the half that was already fast.
 *
 * Size is deliberately NOT a factor. Build output is content-hashed and served
 * immutable, so warming a heavy route costs its bytes once per browser and
 * makes every later visit to it instant; skipping it would trade a permanent
 * win for a one-time saving, and would skip exactly the routes slowest to
 * fetch on demand. The warm is deferred to the load event, so those bytes
 * never compete with the current page's own render.
 *
 * An href matching no page route (an API path, a route this build doesn't
 * serve) yields the union of every route's chunks: no destination is known,
 * but those are needed by any navigation.
 */
export function prefetchTargets(
  manifest: MatchableManifest | null | undefined,
  pathname: string,
): PrefetchTargets {
  if (!manifest || !manifest.routes) return { file: "", imports: [] };
  const matched = matchRoute(manifest, pathname);
  const route = matched ? manifest.routes[matched.component] : null;
  if (route) {
    // The route's OWN transitive chunks — everything it needs, and nothing
    // belonging to routes the user isn't heading for.
    return {
      file: route.file || "",
      imports: Array.from(new Set(route.imports || [])),
    };
  }
  const imports = new Set<string>();
  for (const r of Object.values(manifest.routes)) {
    for (const i of r?.imports || []) imports.add(i);
  }
  return { file: "", imports: Array.from(imports) };
}
