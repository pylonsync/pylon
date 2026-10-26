// The offline service worker, served at `/pylon-sw.js` for apps that set
// `buildManifest({ offline })`.
//
// The client bundler writes the worker into the client build with this
// build's config inlined (`renderServiceWorker`). The worker body is the
// source of `pylonServiceWorker` itself, serialized with `toString()`, so
// the tests in offline-sw.test.ts run the same code the browser runs. That
// imposes one rule on this file: `pylonServiceWorker` and `classifyRequest`
// may not reference anything outside their own bodies (no imports, no
// module-level helpers).
//
// What it does:
//   - install: precache the build's client files for the offline routes,
//     the web app manifest and the icons, then fetch each static offline
//     route's HTML ("shell") with `X-Pylon-Offline-Shell: 1` and keep it when
//     the runtime answers `Pylon-Offline-Shell: 1` (the HTML is the same for
//     every signed-in user).
//   - navigations: network first. When the network fails (or, for an offline
//     route, takes longer than `navigationTimeoutMs`), answer with the
//     route's shell, else the start URL's shell. The client runtime renders
//     the real URL from the local replica.
//   - build files: cache first. They are content-hashed, so a cached copy
//     is always correct. The one exception is `/_pylon/build/manifest.json`,
//     which has the same URL in every build: network first (with the
//     navigation timeout), the cached copy only when the network fails. A
//     cached copy from this worker's build, given to a page of a newer build,
//     sends the page to route files that no longer exist.
//   - everything else (API, auth, sync, dev endpoints): not intercepted, never
//     cached. Data comes from the sync engine, not the HTTP cache.
//   - activate: delete caches from other builds. A new deploy installs a new
//     worker, which waits until every tab of the app is closed (the next
//     launch) before it takes over, or until the page posts
//     `{ type: "pylon:skip-waiting" }` (`useAppUpdate().apply()` in
//     @pylonsync/react does, after the outbox is empty).
//   - messages: `pylon:session` (the signed-in bit, see below),
//     `pylon:skip-waiting`, and `pylon:version`, which answers on the
//     message's port with `{ type: "pylon:version", version }`. The page asks
//     a waiting worker for its version this way.

/** The build-specific values inlined into `/pylon-sw.js`. */
export interface ServiceWorkerConfig {
  /** Hash of everything below. Cache names carry it. */
  version: string;
  /** The client build's public prefix (`/_pylon/build/`, or a CDN URL). */
  buildPrefix: string;
  /** URLs to keep at install: the offline routes' build files, the web app
   *  manifest, the icons. */
  precache: string[];
  /** Offline routes without dynamic segments: their HTML is kept at install. */
  shells: string[];
  /** Every offline route pattern (`/leads/[id]` included). */
  routes: string[];
  /** Path of the start URL; its shell answers any navigation with no
   *  better match. */
  startPath: string;
  /** How long a navigation to an offline route waits for the network before
   *  the worker answers with the shell. */
  navigationTimeoutMs: number;
}

/** What the worker does with one request. */
export type RequestKind =
  /** A page load: network first, shell when offline. */
  | "navigate"
  /** A file of the client build: cache first. */
  | "build"
  /** The build manifest: network first, the cached copy when offline. */
  | "build-manifest"
  /** The web app manifest or an icon: cache first. */
  | "precached"
  /** Not intercepted, never cached. */
  | "network";

/**
 * Classify one request. Pure; `self.location.origin` is passed in.
 */
export function classifyRequest(
  request: { url: string; method: string; mode: string },
  config: ServiceWorkerConfig,
  origin: string,
): RequestKind {
  if (request.method !== "GET") return "network";
  let url: URL;
  try {
    url = new URL(request.url);
  } catch {
    return "network";
  }
  const sameOrigin = url.origin === origin;
  if (sameOrigin) {
    const path = url.pathname;
    // Data and control endpoints. Data reaches the page through the sync
    // engine; a cached API answer would be a stale copy of the replica, and
    // a cached auth answer could sign in the wrong person.
    const neverCached = [
      "/api/",
      "/auth/",
      "/_pylon/dev/",
      "/studio",
      "/events",
      "/health",
      "/metrics",
      "/.well-known/",
      "/_pylon/version",
    ];
    for (const prefix of neverCached) {
      if (path === prefix.replace(/\/$/, "") || path.startsWith(prefix)) return "network";
    }
    if (path === "/pylon-sw.js") return "network";
    if (request.mode === "navigate") return "navigate";
    if (path === "/_pylon/build/manifest.json") return "build-manifest";
    if (path.startsWith("/_pylon/build/")) return "build";
  }
  if (!sameOrigin && config.buildPrefix && url.href.startsWith(config.buildPrefix)) {
    return "build";
  }
  if (sameOrigin && config.precache.indexOf(url.pathname) !== -1) return "precached";
  return "network";
}

/**
 * The worker. `sw` is the ServiceWorkerGlobalScope (`self`); passing it in
 * keeps the body free of globals so tests can supply their own. `classify`
 * is `classifyRequest`, passed in for the same reason.
 */
export function pylonServiceWorker(
  sw: any,
  config: ServiceWorkerConfig,
  classify: typeof classifyRequest,
): void {
  const ASSETS = "pylon-" + config.version + "-assets";
  const SHELLS = "pylon-" + config.version + "-shells";
  const STATE_KEY = "/__pylon_sw_state";
  const SHELL_REQUEST_HEADER = "X-Pylon-Offline-Shell";
  const SHELL_RESPONSE_HEADER = "Pylon-Offline-Shell";

  function warn(message: string): void {
    try {
      sw.console.warn("[pylon offline] " + message);
    } catch {
      // No console in this scope.
    }
  }

  function splitPath(p: string): string[] {
    const trimmed = p.replace(/\/+$/, "");
    return trimmed === "" ? [] : trimmed.slice(1).split("/");
  }

  // Does `pathname` match a route pattern (`/leads/[id]`, `/docs/[...slug]`)?
  function matchesPattern(pattern: string, pathname: string): boolean {
    const pat = splitPath(pattern);
    const target = splitPath(pathname);
    for (let i = 0; i < pat.length; i++) {
      const seg = pat[i];
      if (seg.startsWith("[...") && seg.endsWith("]")) return target.length > i;
      if (seg.startsWith("[[...") && seg.endsWith("]]")) return true;
      if (i >= target.length) return false;
      if (seg.startsWith("[") && seg.endsWith("]")) continue;
      if (seg !== target[i]) return false;
    }
    return pat.length === target.length;
  }

  function isOfflineRoute(pathname: string): boolean {
    for (const route of config.routes) {
      if (matchesPattern(route, pathname)) return true;
    }
    return false;
  }

  function normalizePath(pathname: string): string {
    return pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname;
  }

  // Fetch each static offline route's HTML and keep the ones the runtime
  // marks as identical for every signed-in user. Returns false when the
  // network failed, so the caller does not record the refresh as done.
  async function refreshShells(): Promise<boolean> {
    const cache = await sw.caches.open(SHELLS);
    let reached = false;
    for (const path of config.shells) {
      let res: any;
      try {
        res = await sw.fetch(path, {
          headers: { [SHELL_REQUEST_HEADER]: "1" },
          credentials: "same-origin",
          cache: "no-store",
        });
      } catch {
        continue;
      }
      reached = true;
      const verdict = res.headers.get(SHELL_RESPONSE_HEADER);
      if (res.ok && !res.redirected && verdict === "1") {
        await cache.put(path, res);
      } else if (res.redirected || (verdict && verdict !== "1") || res.status === 404) {
        // A definite answer that this page is not a shell for the current
        // session (a redirect to sign-in, a page that reads the user): drop
        // the old copy so it is not shown to the wrong session. A server
        // error keeps the old copy.
        await cache.delete(path);
        warn(
          path +
            " is not kept for offline use: " +
            (verdict || (res.redirected ? "redirected to " + res.url : "status " + res.status)),
        );
      } else {
        warn(path + " is not kept for offline use: status " + res.status);
      }
    }
    return reached;
  }

  // The client posts its signed-in state after every page load. The shells
  // carry that one bit, so refetch them when it changed.
  async function syncSession(signedIn: boolean): Promise<void> {
    const cache = await sw.caches.open(SHELLS);
    const stored = await cache.match(STATE_KEY);
    if (stored) {
      const state = await stored.json();
      if (state && state.signedIn === signedIn) return;
    }
    if (await refreshShells()) {
      await cache.put(
        STATE_KEY,
        new sw.Response(JSON.stringify({ signedIn }), {
          headers: { "Content-Type": "application/json" },
        }),
      );
    }
  }

  async function shellFor(pathname: string): Promise<any> {
    const cache = await sw.caches.open(SHELLS);
    const exact = await cache.match(normalizePath(pathname));
    if (exact) return exact;
    return cache.match(config.startPath);
  }

  function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = sw.setTimeout(() => reject(new Error("timeout")), ms);
      promise.then(
        (v) => {
          sw.clearTimeout(timer);
          resolve(v);
        },
        (e) => {
          sw.clearTimeout(timer);
          reject(e);
        },
      );
    });
  }

  async function handleNavigation(request: any): Promise<any> {
    const pathname = new URL(request.url).pathname;
    const offlineRoute = isOfflineRoute(pathname);
    const network = sw.fetch(request);
    try {
      // Only an offline route has a page to show instead, so only an
      // offline route stops waiting for a slow network.
      return await (offlineRoute
        ? withTimeout(network, config.navigationTimeoutMs)
        : network);
    } catch (err) {
      const shell = await shellFor(pathname);
      if (shell) return shell;
      // Nothing kept: wait for the network after all (a slow answer beats
      // the browser's offline page), and let a real failure through.
      return network;
    }
  }

  async function cacheFirst(request: any): Promise<any> {
    const cache = await sw.caches.open(ASSETS);
    const hit = await cache.match(request);
    if (hit) return hit;
    const res = await sw.fetch(request);
    // Keep build files the page loads lazily (a dynamic import, an image
    // the page imports), so they work offline after their first use.
    if (res && res.ok) await cache.put(request, res.clone());
    return res;
  }

  // The build manifest has one URL for every build: ask the server first, so
  // a page gets the manifest of the build the server runs. The copy kept at
  // install answers only when the network fails or is slow.
  async function networkFirst(request: any): Promise<any> {
    const cache = await sw.caches.open(ASSETS);
    const network = sw.fetch(request);
    try {
      const res = await withTimeout<any>(network, config.navigationTimeoutMs);
      if (res && res.ok) await cache.put(request, res.clone());
      return res;
    } catch (err) {
      const hit = await cache.match(request);
      if (hit) return hit;
      return network;
    }
  }

  sw.addEventListener("install", (event: any) => {
    event.waitUntil(
      (async () => {
        const cache = await sw.caches.open(ASSETS);
        // All-or-nothing: a build file that fails to download fails the
        // install, and the previous worker keeps serving the previous build.
        await cache.addAll(config.precache);
        await refreshShells();
      })(),
    );
  });

  sw.addEventListener("activate", (event: any) => {
    event.waitUntil(
      (async () => {
        const names: string[] = await sw.caches.keys();
        await Promise.all(
          names
            .filter(
              (n) => n.indexOf("pylon-") === 0 && n !== ASSETS && n !== SHELLS,
            )
            .map((n) => sw.caches.delete(n)),
        );
        // Control the tab that installed the first worker, so the app works
        // offline after one visit. With an older worker this runs only after
        // every tab closed, so it never swaps a build mid-session.
        await sw.clients.claim();
      })(),
    );
  });

  sw.addEventListener("fetch", (event: any) => {
    const request = event.request;
    const kind = classify(request, config, sw.location.origin);
    if (kind === "navigate") event.respondWith(handleNavigation(request));
    else if (kind === "build-manifest") event.respondWith(networkFirst(request));
    else if (kind === "build" || kind === "precached") event.respondWith(cacheFirst(request));
  });

  sw.addEventListener("message", (event: any) => {
    const data = event.data;
    if (!data) return;
    if (data.type === "pylon:session" && typeof data.signedIn === "boolean") {
      event.waitUntil(syncSession(data.signedIn));
    } else if (data.type === "pylon:skip-waiting") {
      event.waitUntil(sw.skipWaiting());
    } else if (data.type === "pylon:version") {
      const port = event.ports && event.ports[0];
      if (port) port.postMessage({ type: "pylon:version", version: config.version });
    }
  });
}

/** The `/pylon-sw.js` source for one build. */
export function renderServiceWorker(config: ServiceWorkerConfig): string {
  return (
    "// Generated by Pylon for this build. Do not edit.\n(" +
    pylonServiceWorker.toString() +
    ")(self, " +
    JSON.stringify(config) +
    ", " +
    classifyRequest.toString() +
    ");\n"
  );
}
