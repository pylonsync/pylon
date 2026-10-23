// The client build files one page needs, from the bundle manifest.
//
// One list, two readers: the SSR render emits a tag for each file (stylesheet
// links, modulepreloads, the polyfill and entry scripts, font preloads), and
// the offline service worker precaches the same files for each offline route.
// Computing them in one place keeps the two from drifting: a file the page
// loads but the worker did not keep would break the page with no network.

/** The bundle-manifest fields this module reads. */
export interface RouteAssetManifest {
  public_prefix?: string;
  polyfills?: string;
  routes: Record<
    string,
    { file: string; imports?: string[]; css?: string[]; path?: string } | undefined
  >;
  fonts?: { preload?: string[] } | null;
}

/** Outdir-relative files for one route, in the order the page loads them. */
export interface RouteAssetFiles {
  /** `<link rel="stylesheet">` (or inlined `<style>`) in the head. */
  styles: string[];
  /** `<link rel="modulepreload">` in the head: the route entry, then its
   *  static import graph (see `modulepreloadFiles`). */
  preloads: string[];
  /** `<link rel="preload" as="font">` in the head. */
  fonts: string[];
  /** `<script type="module">` in the body tail: the polyfills, then the
   *  route entry. */
  scripts: string[];
}

/**
 * The most `<link rel="modulepreload">` tags a page's head carries. Every
 * file in the route's static graph is needed before hydration, so the list
 * is complete for any real route; the cap keeps a pathological graph from
 * bloating the head. Past it, the browser finds the rest by import.
 */
export const MAX_MODULEPRELOADS = 64;

/**
 * The files to modulepreload for a route: its entry first, then its static
 * import graph (the manifest's `imports`, nearest first), capped at
 * `MAX_MODULEPRELOADS`. The entry's `<script>` sits at the end of the body,
 * after a streamed page's Suspense boundaries, so without its preload the
 * page's own code started last.
 */
export function modulepreloadFiles(route: { file: string; imports?: string[] }): string[] {
  return [route.file, ...(route.imports ?? [])].slice(0, MAX_MODULEPRELOADS);
}

/** The scripts the hydration tail loads for a route entry. */
export function routeScriptFiles(
  route: { file: string },
  polyfills: string | undefined,
): string[] {
  return polyfills ? [polyfills, route.file] : [route.file];
}

/** Every client-build file the page for `component` loads, or null when the
 *  build has no entry for it. */
export function routeAssetFiles(
  manifest: RouteAssetManifest,
  component: string,
): RouteAssetFiles | null {
  const route = manifest.routes[component];
  if (!route) return null;
  return {
    styles: route.css ?? [],
    preloads: modulepreloadFiles(route),
    fonts: manifest.fonts?.preload ?? [],
    scripts: routeScriptFiles(route, manifest.polyfills),
  };
}

/** All of a route's files as URLs under the build's public prefix. */
export function routeAssetUrls(
  manifest: RouteAssetManifest,
  component: string,
): string[] {
  const files = routeAssetFiles(manifest, component);
  if (!files) return [];
  const prefix = manifest.public_prefix || "/_pylon/build/";
  // The entry is both preloaded and loaded as a script: list it once.
  return [...new Set([...files.styles, ...files.preloads, ...files.fonts, ...files.scripts])].map(
    (f) => prefix + f,
  );
}
