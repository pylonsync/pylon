// Build-time and render-time pieces of `buildManifest({ offline })`.
//
//   - `readManifestOffline`: the `offline` block of pylon.manifest.json.
//   - `buildOfflineWorker`: computes the service worker config for one client
//     build (the precache list comes from `routeAssetUrls`, the same list the
//     SSR render emits tags for) and writes `pylon-sw.js` into the build,
//     with `pylon-version.json` beside it (served at `/_pylon/version`).
//   - `offlineHeadTags`: the install tags every page's `<head>` gets.
//
// The worker itself is offline-sw.ts. The Rust host serves
// `/manifest.webmanifest`, `/pylon-sw.js` and `/_pylon/version`
// (crates/runtime/src/frontend/offline.rs).

import { renderServiceWorker, type ServiceWorkerConfig } from "./offline-sw";
import { matchRoute } from "./ssr-route-match";
import { routeAssetUrls, type RouteAssetManifest } from "./ssr-route-assets";

/** `buildManifest({ offline })` as the SDK writes it to pylon.manifest.json. */
export interface ManifestOfflineInput {
  name: string;
  shortName?: string;
  description?: string;
  icons?: Array<{ src: string; sizes: string; type?: string; purpose?: string }>;
  themeColor?: string;
  backgroundColor?: string;
  startUrl: string;
  display: string;
  routes: string[];
}

/** What the bundle manifest keeps for the render and the client runtime. */
export interface BundleOffline {
  name: string;
  shortName?: string;
  themeColor?: string;
  icons?: Array<{ src: string; sizes: string; type?: string; purpose?: string }>;
  startUrl: string;
  routes: string[];
  /** The service worker's version (its cache names carry it). */
  version: string;
  /** When this build was made (ISO 8601). Not part of `version`: two
   *  machines that build the same app get the same version. */
  builtAt: string;
}

/** `pylon-version.json` in the client build, served at `/_pylon/version`.
 *  `useAppUpdate` in @pylonsync/react polls it. */
export interface BuildVersionFile {
  version: string;
  builtAt: string;
  /** Hash of what a home-screen app reads only when it is added (see
   *  `installIdentity`). */
  install: string;
}

/** The file the runtime serves at `/_pylon/version`. */
export const VERSION_FILE = "pylon-version.json";

/** How long a navigation to an offline route waits for the network before
 *  the worker shows the kept page. Long enough for a slow but working
 *  connection to win; short enough that a dead one does not leave a blank
 *  screen. */
export const NAVIGATION_TIMEOUT_MS = 3000;

/** The `offline` block of `<cwd>/pylon.manifest.json`, or null. */
export function readManifestOffline(
  fs: any,
  path: any,
  cwd: string,
  manifestPath: string = path.join(cwd, "pylon.manifest.json"),
): ManifestOfflineInput | null {
  if (!fs.existsSync(manifestPath)) return null;
  try {
    const m = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    const o = m?.offline;
    if (!o || typeof o.name !== "string") return null;
    return {
      ...o,
      startUrl: typeof o.startUrl === "string" ? o.startUrl : "/",
      display: typeof o.display === "string" ? o.display : "standalone",
      routes: Array.isArray(o.routes) ? o.routes : [],
    };
  } catch {
    return null;
  }
}

/** Every route pattern in `routes` that names a page in this build, with
 *  the page's component. A pattern with no page is reported, not dropped
 *  silently. */
export function resolveOfflineRoutes(
  manifest: RouteAssetManifest,
  routes: string[],
): { matched: Array<{ pattern: string; component: string }>; unknown: string[] } {
  const matched: Array<{ pattern: string; component: string }> = [];
  const unknown: string[] = [];
  for (const pattern of routes) {
    const component = Object.keys(manifest.routes).find(
      (c) => manifest.routes[c]?.path === pattern,
    );
    if (component) {
      matched.push({ pattern, component });
      continue;
    }
    // A concrete path (`/leads/today`) served by a dynamic page.
    const m = matchRoute(manifest as any, pattern);
    if (m) matched.push({ pattern, component: m.component });
    else unknown.push(pattern);
  }
  return { matched, unknown };
}

/** True for a pattern with a dynamic segment: one shell cannot serve it. */
export function isDynamicPattern(pattern: string): boolean {
  return /\[[^\]]+\]/.test(pattern);
}

/**
 * Hash of what iOS reads from the web app manifest and the head tags only
 * when the app is added to the home screen: the name, the short name, the
 * display mode, the start URL and the icons (with the bytes of each icon
 * file in `public/`). When it changes, an installed app keeps the old
 * values until it is removed and added again.
 *
 * The manifest is the one the runtime serves: `public/manifest.webmanifest`,
 * then `<appDir>/manifest.webmanifest`, else the `offline` config (see
 * `web_manifest_source` in crates/runtime/src/frontend/offline.rs).
 */
export async function installIdentity(
  fs: any,
  path: any,
  cwd: string,
  appDirRel: string,
  offline: ManifestOfflineInput,
): Promise<string> {
  let served: any = null;
  for (const candidate of [
    path.join(cwd, "public", "manifest.webmanifest"),
    path.join(cwd, appDirRel, "manifest.webmanifest"),
  ]) {
    if (!fs.existsSync(candidate)) continue;
    try {
      served = JSON.parse(fs.readFileSync(candidate, "utf8"));
    } catch {
      served = { unreadable: fs.readFileSync(candidate, "utf8") };
    }
    break;
  }
  const { createHash } = await import("node:crypto");
  const iconContent = (src: unknown): string | null => {
    if (typeof src !== "string" || !src.startsWith("/") || src.startsWith("//")) return null;
    const file = path.join(cwd, "public", src.split(/[?#]/)[0]);
    if (!fs.existsSync(file)) return null;
    return createHash("sha256").update(fs.readFileSync(file)).digest("hex");
  };
  const icons = (list: unknown) =>
    (Array.isArray(list) ? list : []).map((icon: any) => ({
      src: icon?.src ?? null,
      sizes: icon?.sizes ?? null,
      purpose: icon?.purpose ?? null,
      content: iconContent(icon?.src),
    }));
  const identity = served
    ? {
        name: served.name ?? null,
        shortName: served.short_name ?? null,
        display: served.display ?? null,
        startUrl: served.start_url ?? null,
        icons: icons(served.icons),
        unreadable: served.unreadable ?? null,
      }
    : {
        name: offline.name,
        shortName: offline.shortName ?? offline.name,
        display: offline.display,
        startUrl: offline.startUrl,
        icons: icons(offline.icons),
      };
  // The head tags come from the `offline` config even when the app ships its
  // own manifest file.
  const touch = touchIcon(offline.icons);
  const head = {
    title: offline.shortName ?? offline.name,
    touchIcon: touch,
    touchIconContent: iconContent(touch),
  };
  return createHash("sha256")
    .update(JSON.stringify({ identity, head }))
    .digest("hex")
    .slice(0, 16);
}

/** The service worker config for this build. `install` is
 *  `installIdentity`; it is part of the version, so a manifest change
 *  installs a new worker (which precaches the new manifest). */
export async function offlineWorkerConfig(
  manifest: RouteAssetManifest,
  offline: ManifestOfflineInput,
  install: string = "",
): Promise<{ config: ServiceWorkerConfig; unknownRoutes: string[] }> {
  const prefix = manifest.public_prefix || "/_pylon/build/";
  const { matched, unknown } = resolveOfflineRoutes(manifest, offline.routes);
  const precache = new Set<string>();
  // The client runtime loads the bundle manifest before it can render a
  // route; it is always same-origin.
  precache.add("/_pylon/build/manifest.json");
  for (const { component } of matched) {
    for (const url of routeAssetUrls(manifest, component)) precache.add(url);
  }
  precache.add("/manifest.webmanifest");
  for (const icon of offline.icons ?? []) precache.add(icon.src);
  const startPath = offline.startUrl.split(/[?#]/)[0] || "/";
  const shells = matched
    .map((m) => m.pattern)
    .filter((p) => !isDynamicPattern(p));
  if (!shells.includes(startPath)) shells.unshift(startPath);
  const base = {
    buildPrefix: prefix,
    precache: Array.from(precache).sort(),
    shells,
    routes: offline.routes,
    startPath,
    navigationTimeoutMs: NAVIGATION_TIMEOUT_MS,
  };
  // Content-derived: the build files are content-hashed, so the same build
  // gives the same version on every machine and every boot. A version that
  // changed per boot would reinstall the worker on every deploy machine.
  const { createHash } = await import("node:crypto");
  const version = createHash("sha256")
    .update(JSON.stringify({ ...base, install }))
    .digest("hex")
    .slice(0, 16);
  return { config: { version, ...base }, unknownRoutes: unknown };
}

/** Write `pylon-sw.js` and `pylon-version.json` into `outdir` and return
 *  the bundle manifest's `offline` block. `cwd` and `appDirRel` locate the
 *  app's own manifest file and its icons. */
export async function buildOfflineWorker(
  fs: any,
  path: any,
  outdir: string,
  manifest: RouteAssetManifest,
  offline: ManifestOfflineInput,
  cwd: string,
  appDirRel: string,
): Promise<BundleOffline> {
  const install = await installIdentity(fs, path, cwd, appDirRel, offline);
  const { config, unknownRoutes } = await offlineWorkerConfig(manifest, offline, install);
  if (unknownRoutes.length > 0) {
    // eslint-disable-next-line no-console
    console.warn(
      `[pylon offline] offline.routes names no page for: ${unknownRoutes.join(", ")}. ` +
        "Those paths will not open offline.",
    );
  }
  fs.writeFileSync(path.join(outdir, "pylon-sw.js"), renderServiceWorker(config), "utf8");
  const versionFile: BuildVersionFile = {
    version: config.version,
    builtAt: new Date().toISOString(),
    install,
  };
  fs.writeFileSync(path.join(outdir, VERSION_FILE), JSON.stringify(versionFile), "utf8");
  return {
    name: offline.name,
    ...(offline.shortName ? { shortName: offline.shortName } : {}),
    ...(offline.themeColor ? { themeColor: offline.themeColor } : {}),
    ...(offline.icons && offline.icons.length > 0 ? { icons: offline.icons } : {}),
    startUrl: offline.startUrl,
    routes: offline.routes,
    version: config.version,
    builtAt: versionFile.builtAt,
  };
}

function attr(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** The largest square icon, for `apple-touch-icon` (iOS does not read the
 *  web app manifest's icons). */
function touchIcon(icons: BundleOffline["icons"]): string | null {
  let best: { src: string; size: number } | null = null;
  for (const icon of icons ?? []) {
    if (icon.purpose && !icon.purpose.split(" ").includes("any")) continue;
    for (const s of icon.sizes.split(" ")) {
      const m = s.match(/^(\d+)x(\d+)$/);
      const size = m && m[1] === m[2] ? Number(m[1]) : 0;
      if (size > 0 && (!best || size > best.size)) best = { src: icon.src, size };
    }
  }
  return best ? best.src : null;
}

/** `<head>` tags that make the app installable. */
export function offlineHeadTags(offline: BundleOffline): string {
  let tags = `<link rel="manifest" href="/manifest.webmanifest">`;
  if (offline.themeColor) {
    tags += `<meta name="theme-color" content="${attr(offline.themeColor)}">`;
  }
  tags += `<meta name="mobile-web-app-capable" content="yes">`;
  tags += `<meta name="apple-mobile-web-app-capable" content="yes">`;
  tags += `<meta name="apple-mobile-web-app-title" content="${attr(offline.shortName ?? offline.name)}">`;
  const icon = touchIcon(offline.icons);
  if (icon) tags += `<link rel="apple-touch-icon" href="${attr(icon)}">`;
  return tags;
}
