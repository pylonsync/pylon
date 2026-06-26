// Build the client-side hydration bundle for a Pylon SSR project.
//
// Phase 1.5e: code splitting with shared chunks. The bundler:
//
//   1. Discovers `app/**/page.tsx` + `app/**/layout.tsx` under cwd.
//   2. Generates one tiny `client-runtime.ts` module containing the
//      hydration dispatcher + React imports (the *only* place that
//      pulls in react-dom/client).
//   3. Generates one per-route entry — `client-entry-<slug>.tsx` —
//      that statically imports its page + its layout chain, then
//      calls into the runtime.
//   4. Hands every per-route entry to `Bun.build` with
//      `splitting: true` + `metafile: true`. Bun's splitter sees
//      `client-runtime` (and thus React) imported by every entry
//      and extracts it into a shared chunk under `chunks/`.
//   5. Walks `metafile.outputs` to emit a `manifest.json` keyed on
//      the project-relative component path the SSR side already
//      uses. The manifest lists the route's entry file + every
//      transitive `import` chunk so the SSR HTML head can emit the
//      right `<script type=module>` + `<link rel=modulepreload>`
//      pair.
//
// Result for `examples/ssr-hello` (2 routes, 1 layout): one shared
// `chunks/chunk-*.js` carrying React (~120KB gz), plus two tiny
// per-route entries (~1-2KB each). A visit to /hello loads the
// shared chunk + the /hello entry; a subsequent click to / hits the
// cache for the shared chunk and only pulls the / entry.
//
// What it doesn't do (Phase 1.5f+):
//   - File-watcher invalidation. Rebuild requires pylon dev restart.
//   - Source maps. Will enable in dev once the basics are solid.
//   - Link prefetching. <PylonLink> with IntersectionObserver
//     prefetch is a follow-up — splitting is the precondition.
//   - CSS chunking. No CSS support in SSR yet.

import { buildFonts, readManifestFonts, type ManifestFonts } from "./ssr-fonts";

type Send = (msg: Record<string, unknown>) => void;

interface BundleClientMessage {
  type: "bundle_client";
  call_id: string;
  /**
   * Project-relative directory holding the route tree
   * (`<app_dir>/**​/page.tsx`). Defaults to `"app"` when the host
   * doesn't send it (older hosts, or the default single-`app/`
   * layout). The full-stack app that namespaces its frontend under a
   * subdir — e.g. `web/app` via `discoverAppRoutes({appDir:"web/app"})`
   * — sends the same dir here so the client bundler and the SSR
   * manifest agree on where the routes live.
   */
  app_dir?: string;
}

interface DiscoveredRoute {
  /**
   * Project-relative module path without extension. This is the
   * key the SSR side passes in __PYLON_DATA__.component and the key
   * the manifest is indexed by.
   */
  component: string;
  /** Layout chain root → leaf, same format as `component`. */
  layouts: string[];
}

/** Bun.build returns this shape (the subset we depend on). */
type BunBuildOutput = {
  success: boolean;
  outputs: Array<{
    path: string;
    kind: string;
    hash?: string;
    text?(): Promise<string>;
  }>;
  logs?: Array<{ level: string; message: string }>;
};

declare const Bun: {
  build(opts: {
    entrypoints: string[];
    outdir?: string;
    target?: "browser" | "bun" | "node";
    format?: "esm" | "iife";
    minify?: boolean;
    sourcemap?: "none" | "inline" | "external";
    define?: Record<string, string>;
    external?: string[];
    splitting?: boolean;
    naming?:
      | string
      | {
          entry?: string;
          chunk?: string;
          asset?: string;
        };
    publicPath?: string;
    root?: string;
  }): Promise<BunBuildOutput>;
  file(path: string): { exists(): Promise<boolean> };
};

/**
 * Synchronously walk the route dir (`<appDirRel>` under cwd, e.g.
 * `app` or `web/app`) and return one entry per discovered page, each
 * carrying its layout chain (root → leaf). `appDirRel` MUST match the
 * `appDir` the manifest was built with (`discoverAppRoutes({appDir})`)
 * so the component paths — `path.relative(cwd, file)` — line up with
 * the manifest's `component` field byte-for-byte. Mirrors the
 * discovery logic in @pylonsync/sdk's `discoverAppRoutes` exactly:
 * same sort order, same group-strip.
 */
function discoverRoutes(
  fs: any,
  path: any,
  cwd: string,
  appDirRel: string,
): DiscoveredRoute[] {
  const appDir = path.join(cwd, appDirRel);
  if (!fs.existsSync(appDir) || !fs.statSync(appDir).isDirectory()) {
    return [];
  }

  type PageHit = { segments: string[]; component: string; layouts: string[] };
  const pages: PageHit[] = [];

  function walk(dir: string, segments: string[], layouts: string[]): void {
    let entries: Array<{ name: string; isDirectory(): boolean }>;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    const layoutHere = ["layout.tsx", "layout.ts", "layout.jsx", "layout.js"]
      .map((n: string) => path.join(dir, n))
      .find((p: string) => fs.existsSync(p));
    const nextLayouts = layoutHere
      ? [...layouts, path.relative(cwd, layoutHere).replace(/\.(tsx?|jsx?)$/, "")]
      : layouts;
    const pageHere = ["page.tsx", "page.ts", "page.jsx", "page.js"]
      .map((n: string) => path.join(dir, n))
      .find((p: string) => fs.existsSync(p));
    if (pageHere) {
      pages.push({
        segments: [...segments],
        component: path.relative(cwd, pageHere).replace(/\.(tsx?|jsx?)$/, ""),
        layouts: nextLayouts,
      });
    }
    // Boundary modules (not-found.tsx / error.tsx) are hydrated like pages
    // (#279) so onClick/useState/reset() work — that means each needs its own
    // client entry + manifest key, keyed by component path exactly like a page.
    // They wrap in the layouts ABOVE them (nextLayouts), same as a page here.
    for (const base of ["not-found", "error"]) {
      const bHere = [`${base}.tsx`, `${base}.ts`, `${base}.jsx`, `${base}.js`]
        .map((n: string) => path.join(dir, n))
        .find((p: string) => fs.existsSync(p));
      if (bHere) {
        pages.push({
          segments: [...segments],
          component: path.relative(cwd, bHere).replace(/\.(tsx?|jsx?)$/, ""),
          layouts: nextLayouts,
        });
      }
    }
    for (const e of entries) {
      if (!e.isDirectory()) continue;
      if (e.name.startsWith(".") || e.name === "node_modules") continue;
      const sub = path.join(dir, e.name);
      const isGroup = e.name.startsWith("(") && e.name.endsWith(")");
      const newSegments = isGroup ? segments : [...segments, e.name];
      walk(sub, newSegments, nextLayouts);
    }
  }
  walk(appDir, [], []);

  return pages.map((p) => ({
    component: p.component,
    layouts: p.layouts,
  }));
}

/**
 * The shared hydration dispatcher + router. ONE module, imported
 * by every per-route entry. Bun's splitter sees N entries reach
 * for it and pulls it (and React via the transitive imports) into
 * a shared chunk.
 *
 * Two responsibilities:
 *   1. `hydrate(component, Page, Layouts)` — called by each route
 *      entry. The first call (matching the SSR'd component) calls
 *      hydrateRoot; subsequent calls (after a client-side
 *      navigation) re-render the cached root with the new tree.
 *      Either way, the entry also registers itself in the route
 *      cache so future navigations don't need to refetch the chunk.
 *   2. `navigate(href)` — fetches the new SSR HTML, parses out the
 *      `__PYLON_DATA__` payload, dynamically loads the new route's
 *      entry from the manifest, and re-renders into the existing
 *      React root. Layouts that match the previous render survive
 *      reconciliation (React keeps their state).
 *
 * Global click handler intercepts `a[data-pylon-link]` clicks for
 * client-side nav. popstate handles back/forward. The runtime
 * exposes `window.__pylon` for the `<Link>` component to call
 * directly (prefetch, navigate).
 */
const CLIENT_RUNTIME_SOURCE = `// Generated by Pylon SSR (Phase 2 client runtime).
// DO NOT EDIT — overwritten on every pylon dev / build.

import { createElement } from "react";
import { hydrateRoot } from "react-dom/client";
import { createPylonBoundary } from "./client-boundary";

const routeCache = Object.create(null);
let activeRoot = null;
// Destination of an in-flight client navigation. Read by hydrateRoot's
// onUncaughtError so a re-render that throws mid-nav degrades to a full page
// load instead of a white screen. Null when no nav is in flight.
let pendingNav = null;
let manifestPromise = null;
const prefetchedChunks = new Set();

function buildTree(Page, Layouts, props) {
  let tree = createElement(Page, props);
  for (let i = Layouts.length - 1; i >= 0; i--) {
    const Layout = Layouts[i];
    if (!Layout) continue;
    tree = createElement(Layout, props, tree);
  }
  return tree;
}

// ---------------------------------------------------------------------------
// Client error / not-found boundary. notFound() (from @pylonsync/react) and
// any other error thrown DURING a client render — the normal case when an
// async lookup 404s after hydration — would otherwise propagate uncaught and
// blank the whole document. React error boundaries must be class components;
// this one catches the throw, resolves the nearest not-found.tsx / error.tsx
// for the active route from the build manifest (the same nearest-ancestor
// model the server's findBoundary uses), and renders it in its own layout
// chain. It sits at the root and is transparent until something throws, so
// hydration still matches the server HTML exactly. It resets on navigation
// via a changing navEpoch prop (NOT a key — a key change would remount every
// layout on each nav and lose their state).
// ---------------------------------------------------------------------------

let navEpoch = 0;
// The props the active page was rendered with (auth, serverData, params, …).
// The boundary reuses them so a boundary module AND its layout chain — e.g. an
// auth-guarding dashboard layout that reads props.auth — render with the SAME
// context the page had. The server renders boundaries with the page props too;
// a minimal hand-built props object would make such a layout see no auth and
// (for instance) redirect to /login instead of showing the not-found page.
let currentPageProps = {};

// The not-found / error boundary lives in a real module (./client-boundary,
// emitted next to this file by the bundler) so its render behavior is
// unit-testable instead of buried in this string. Wire the runtime's
// internals in as deps; the returned withBoundary(tree, component, navEpoch)
// wraps every page tree as the transparent root error boundary.
const { withBoundary } = createPylonBoundary({
  loadManifest,
  loadRouteEntry,
  navigate,
  buildTree,
  getPageProps: () => currentPageProps,
  getResetHref: () => location.pathname + location.search,
});

// Deterministic stringify — MUST match stableStringify in ssr-runtime.ts so
// a serverData call's cache key is identical on server and client.
function stableStringify(v) {
  if (v === null || v === undefined || typeof v !== "object") {
    return JSON.stringify(v === undefined ? null : v);
  }
  if (Array.isArray(v)) return "[" + v.map(stableStringify).join(",") + "]";
  const keys = Object.keys(v).sort();
  return (
    "{" +
    keys.map((k) => JSON.stringify(k) + ":" + stableStringify(v[k])).join(",") +
    "}"
  );
}

const SERVER_DATA_METHODS = [
  "get",
  "list",
  "lookup",
  "query",
  "queryGraph",
  "paginate",
  "search",
];

// A React-recognized fulfilled thenable: use() reads .value synchronously
// (status === "fulfilled") instead of suspending. Critical for hydration —
// the server streamed the post-Suspense content, so the client must render
// that content on the FIRST pass without re-suspending (no fallback flash,
// no mismatch).
function fulfilledThenable(value) {
  return {
    status: "fulfilled",
    value,
    then(onFulfilled) {
      return onFulfilled ? onFulfilled(value) : value;
    },
  };
}

// Client stand-in for the server's serverData handle. Each method returns a
// pre-fulfilled thenable (cached per key) sourced from the SSR'd ssrData map,
// keyed identically to the server. Misses yield undefined — the page
// rendered with whatever the server fetched, so a hit is expected.
function makeClientServerData(ssrData) {
  const cache = ssrData || {};
  const pc = new Map();
  const wrap = (prefix) => {
    const out = {};
    for (const m of SERVER_DATA_METHODS) {
      out[m] = (...args) => {
        const key = prefix + m + ":" + stableStringify(args);
        if (!pc.has(key)) pc.set(key, fulfilledThenable(cache[key]));
        return pc.get(key);
      };
    }
    return out;
  };
  const sd = wrap("");
  sd.unsafe = wrap("u:");
  return sd;
}

// Server-only response controller has no meaning on the client (the status/
// redirect/cookies already shipped). Give pages a no-op so a body that
// touches props.response during hydration doesn't crash.
function makeNoopResponse() {
  const noop = () => {};
  return {
    setStatus: noop,
    setHeader: noop,
    setCookie: noop,
    redirect: noop,
    notFound: noop,
  };
}

// Rehydrate the live, server-only props (serverData + response) that were
// stripped before serialization, so the client tree matches the server's.
// For a hydrated error boundary (#279), synthesize the reset() the server
// rendered as a no-op: re-fetch + re-render the current URL (a transient
// error clears to the page; a deterministic one re-shows the boundary).
// The current route's dynamic params (e.g. { projectId: "p_1" }). Lives here,
// not in the DOM __PYLON_DATA__ (which navigate() never rewrites), so useParams()
// in a deep client child has a reactive source. A fresh object per nav → stable
// reference between navs (useSyncExternalStore needs that).
let currentParams = {};
function setNavParams(data) {
  currentParams = (data && data.props && data.props.params) || {};
}

function withClientProps(data) {
  const props = { ...(data.props || {}) };
  props.serverData = makeClientServerData(data.ssrData);
  props.response = makeNoopResponse();
  if (data.kind === "error") {
    props.reset = function () {
      navigate(location.pathname + location.search, { replace: true });
    };
  }
  return props;
}

function readPylonData() {
  const dataEl = document.getElementById("__PYLON_DATA__");
  if (!dataEl) return null;
  try {
    return JSON.parse(dataEl.textContent || "{}");
  } catch {
    return null;
  }
}

async function loadManifest() {
  if (!manifestPromise) {
    manifestPromise = fetch("/_pylon/build/manifest.json", {
      credentials: "same-origin",
    })
      .then((r) => (r.ok ? r.json() : null))
      .catch(() => null);
  }
  return manifestPromise;
}

function preloadChunks(routeInfo) {
  if (!routeInfo) return;
  const prefix = routeInfo.public_prefix || "/_pylon/build/";
  const all = [routeInfo.file, ...(routeInfo.imports || [])];
  for (const c of all) {
    if (prefetchedChunks.has(c)) continue;
    prefetchedChunks.add(c);
    const link = document.createElement("link");
    link.rel = "modulepreload";
    link.href = prefix + c;
    document.head.appendChild(link);
  }
}

async function prefetch(href) {
  // HTML prefetch — primes the SSR response cache.
  const url = new URL(href, location.href);
  if (url.origin !== location.origin) return;
  if (!document.querySelector('link[rel="prefetch"][href="' + url.pathname + '"]')) {
    const html = document.createElement("link");
    html.rel = "prefetch";
    html.as = "document";
    html.href = url.pathname + url.search;
    document.head.appendChild(html);
  }
  // Chunk prefetch — peek at the manifest, but we don't know the
  // component path from the href without server help. v1: rely on
  // the shared chunk already being cached + the SSR head emitting
  // the right preload tags after the user lands. So all prefetch
  // does today is HTML — chunk dedup happens via the manifest.
  const manifest = await loadManifest();
  if (manifest) {
    // Pre-warm all routes' shared chunks (one chunk in practice).
    const shared = new Set();
    for (const r of Object.values(manifest.routes || {})) {
      for (const i of r.imports || []) shared.add(i);
    }
    const info = { public_prefix: manifest.public_prefix, file: "", imports: Array.from(shared) };
    preloadChunks(info);
  }
}

async function loadRouteEntry(component) {
  if (routeCache[component]) return routeCache[component];
  const manifest = await loadManifest();
  if (!manifest) throw new Error("manifest unavailable");
  const routeInfo = manifest.routes[component];
  if (!routeInfo) throw new Error("unknown component " + component);
  const prefix = manifest.public_prefix || "/_pylon/build/";
  preloadChunks({ ...routeInfo, public_prefix: prefix });
  // Dynamic import the entry. It will call hydrate(...) which
  // populates routeCache[component], then this returns.
  await import(/* @vite-ignore */ prefix + routeInfo.file);
  if (!routeCache[component]) {
    throw new Error("route entry did not register: " + component);
  }
  return routeCache[component];
}

export function hydrate(component, Page, Layouts) {
  // Always cache the route for nav.
  routeCache[component] = { Page, Layouts };
  const data = readPylonData();
  // First hydrate: the entry's component MATCHES the SSR'd page.
  // Establish the root + install the click + popstate handlers
  // exactly once.
  if (!activeRoot) {
    if (!data || data.component !== component) {
      console.warn(
        "[pylon ssr] entry/__PYLON_DATA__ mismatch — initial hydration skipped",
      );
      return;
    }
    setNavParams(data);
    currentPageProps = withClientProps(data);
    const tree = withBoundary(
      buildTree(Page, Layouts, currentPageProps),
      data.component,
      navEpoch,
    );
    activeRoot = hydrateRoot(document, tree, {
      // Safety net for client navigation. If a nav re-render throws an uncaught
      // error in React's commit phase, the URL has already changed but the page
      // can't swap — a white/stale screen. The classic trigger is a page that
      // renders hoisted <title>/<meta>/<link> in its own tree (use the
      // \`metadata\` export instead); React 19 owns those head nodes on the client
      // and reconciling them across routes can throw. Rather than strand the
      // user, fall back to a full page load of the pending destination, which
      // re-renders it cleanly from SSR. Non-navigation errors keep React's
      // default reporting.
      onUncaughtError(error) {
        if (pendingNav) {
          const dest = pendingNav;
          pendingNav = null;
          console.error(
            "[pylon ssr] client navigation failed to render; falling back to a full page load:",
            error,
          );
          window.location.href = dest;
          return;
        }
        console.error(error);
      },
    });
    installNavHandlers();
    return;
  }
  // Subsequent hydrate calls fire from dynamic-loaded entries
  // during navigation. The render is driven by navigate(), so we
  // only need to populate the cache (already done above).
}

// Swap the page's SEO/social <head> tags on a client-side navigation.
// The SSR runtime marks every page-metadata <meta>/<link> with
// data-pylon-meta; we drop the current set and import the incoming page's
// set, so description / canonical / og:* / twitter:* / icons track the new
// route. The layout's charset/viewport and Pylon's injected stylesheet
// links carry no marker, so they survive untouched (no FOUC). The page
// component never renders these tags on the client, so React doesn't own
// them — this manual swap can't fight hydration.
function syncHeadMeta(doc) {
  const head = document.head;
  if (!head) return;
  const current = head.querySelectorAll("[data-pylon-meta]");
  for (let i = 0; i < current.length; i++) current[i].remove();
  const incoming = doc.head
    ? doc.head.querySelectorAll("[data-pylon-meta]")
    : [];
  for (let i = 0; i < incoming.length; i++) {
    head.appendChild(document.importNode(incoming[i], true));
  }
}

async function navigate(href, opts) {
  const push = !opts || opts.push !== false;
  const url = new URL(href, location.href);
  if (url.origin !== location.origin) {
    window.location.href = href;
    return;
  }
  let html;
  try {
    const res = await fetch(url.pathname + url.search, {
      credentials: "same-origin",
      headers: { Accept: "text/html" },
    });
    if (!res.ok) {
      window.location.href = href;
      return;
    }
    html = await res.text();
  } catch {
    window.location.href = href;
    return;
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  const dataEl = doc.getElementById("__PYLON_DATA__");
  if (!dataEl) {
    window.location.href = href;
    return;
  }
  let data;
  try {
    data = JSON.parse(dataEl.textContent || "{}");
  } catch {
    window.location.href = href;
    return;
  }
  let route;
  try {
    route = await loadRouteEntry(data.component);
  } catch (e) {
    console.warn("[pylon ssr] nav fallback (entry load failed):", e);
    window.location.href = href;
    return;
  }
  document.title = doc.title || document.title;
  syncHeadMeta(doc);
  setNavParams(data);
  navEpoch++;
  currentPageProps = withClientProps(data);
  const target = url.pathname + url.search;
  const tree = withBoundary(
    buildTree(route.Page, route.Layouts, currentPageProps),
    data.component,
    navEpoch,
  );
  // Track the in-flight destination so hydrateRoot's onUncaughtError can fall
  // back to a full page load if this re-render throws in React's commit phase
  // (instead of leaving the URL changed but the page unswapped). Cleared on the
  // next macrotask once the commit has settled with no error.
  pendingNav = target;
  activeRoot.render(tree);
  setTimeout(() => {
    if (pendingNav === target) pendingNav = null;
  }, 0);
  if (opts && opts.replace) {
    history.replaceState({ component: data.component }, "", target);
  } else if (push) {
    history.pushState({ component: data.component }, "", target);
  }
  // Notify the router hooks (useSearchParams / usePathname) so deep children
  // re-read location after a Link click or a router.push(). popstate already
  // covers back/forward, but pushState/replaceState fire no event.
  window.dispatchEvent(new Event("pylon:navigation"));
  // After a successful nav, scroll to top (Next.js default).
  window.scrollTo(0, 0);
}

function installNavHandlers() {
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented) return;
    if (e.button !== 0) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const target = e.target;
    if (!target || !target.closest) return;
    const link = target.closest("a[data-pylon-link]");
    if (!link) return;
    const href = link.getAttribute("href");
    if (!href) return;
    if (link.target && link.target !== "" && link.target !== "_self") return;
    if (href.startsWith("http://") || href.startsWith("https://") || href.startsWith("//")) {
      // External — let the browser handle.
      return;
    }
    e.preventDefault();
    navigate(href);
  });
  window.addEventListener("popstate", () => {
    navigate(location.pathname + location.search, { push: false });
  });
  // Progressive-enhancement form submit (#276): intercept <form data-pylon-form>
  // so the page doesn't full-reload. fetch the same route.ts endpoint, follow
  // the handler's redirect, and swap the destination in via navigate(). Without
  // JS this listener never runs and the browser submits natively (POST →
  // handler → 303 → GET) — identical result, just a full navigation.
  document.addEventListener("submit", (e) => {
    if (e.defaultPrevented) return;
    const form = e.target;
    if (!form || form.tagName !== "FORM") return;
    if (!form.hasAttribute("data-pylon-form")) return;
    const action = form.getAttribute("action");
    if (!action) return;
    // Off-origin actions / new-tab targets → let the browser submit.
    if (action.startsWith("http://") || action.startsWith("https://") || action.startsWith("//")) return;
    const tgt = form.getAttribute("target");
    if (tgt && tgt !== "" && tgt !== "_self") return;
    const method = (form.getAttribute("method") || "post").toUpperCase();
    e.preventDefault();
    // urlencoded body (matches the server's parser; files use the native path).
    const body = new URLSearchParams();
    const fd = new FormData(form);
    fd.forEach((v, k) => {
      if (typeof v === "string") body.append(k, v);
    });
    fetch(action, {
      method,
      body,
      credentials: "same-origin",
      headers: { Accept: "text/html" },
    })
      .then((res) => {
        // fetch followed the handler's 303 → res.url is the destination page.
        // Drive a client navigation to it (re-renders without a full reload).
        const dest = new URL(res.url || action, location.href);
        if (dest.origin === location.origin) {
          navigate(dest.pathname + dest.search);
        } else {
          window.location.href = dest.href;
        }
      })
      .catch(() => {
        // Network/abort — fall back to a native submit so the user isn't stuck.
        form.removeAttribute("data-pylon-form");
        form.submit();
      });
  });
}

// Expose for <Link> component prefetch.
const pylonGlobal = {
  prefetch,
  navigate,
  // Read by useParams(); a getter so it always reflects the latest nav.
  get params() {
    return currentParams;
  },
};
if (typeof window !== "undefined") {
  window.__pylon = pylonGlobal;
}
`;

/**
 * Per-route entry source. Stays tiny on purpose — react /
 * react-dom / client-runtime get hoisted into the shared chunk
 * by the splitter, so this body ends up as roughly "load the
 * shared chunk, then call hydrate(Page, [L0, L1, ...])".
 *
 * Each route gets its own entry file under `.pylon/`. The entry
 * path for component `app/hello/page` is
 * `.pylon/client-entry-app__hello__page.tsx` — flat namespace
 * keyed on the slug we'd already need anyway for the manifest.
 */
function generateRouteEntry(route: DiscoveredRoute): string {
  const layoutImports = route.layouts
    .map((l, i) => `import L${i} from "${cwd_to_import(l)}";`)
    .join("\n");
  const layoutArray = route.layouts.map((_, i) => `L${i}`).join(", ");
  return `// Generated by Pylon SSR (Phase 2 per-route entry).
// DO NOT EDIT — overwritten on every pylon dev / build.

import { hydrate } from "./client-runtime";
import Page from "${cwd_to_import(route.component)}";
${layoutImports}

hydrate(${JSON.stringify(route.component)}, Page, [${layoutArray}]);
`;
}

/**
 * Bun's static import-path resolution runs against the source
 * file's directory. Per-route entries live at
 * `<cwd>/.pylon/client-entry-<slug>.tsx`, so reaching
 * `<cwd>/app/page.tsx` is `../app/page`. The shared runtime stays
 * at `./client-runtime` since it sits next to the entries.
 */
function cwd_to_import(modulePath: string): string {
  return `../${modulePath}`;
}

/**
 * Project-relative component path → filename-safe slug.
 * `app/hello/page` → `app__hello__page`. Used for the entry
 * filename and (after Bun appends the hash) for the manifest key
 * mapping back to the component path.
 */
function slugForComponent(component: string): string {
  return component.replace(/[^A-Za-z0-9_]/g, "__");
}

/**
 * Manifest schema. One entry per route, indexed by the same
 * project-relative component path the SSR side passes through.
 * `file` is the entry chunk, `imports` is the transitive set of
 * shared chunks the browser needs to load BEFORE the entry runs
 * — that's the modulepreload list.
 *
 * Paths in the manifest are relative to `.pylon/client-build/` so
 * the Rust host can serve them under `/_pylon/build/<path>` with
 * no rewriting.
 */
export interface PylonBundleManifest {
  /** Build identity — bumps every successful build. */
  build_id: string;
  /** Output root, relative to cwd (always `.pylon/client-build`). */
  outdir: string;
  /** Public URL prefix the Rust host serves chunks under. */
  public_prefix: string;
  /** routeComponentPath → file + imports for that route. */
  routes: Record<
    string,
    {
      /** Per-route entry file, relative to outdir. */
      file: string;
      /** Transitive shared chunks to modulepreload, relative to outdir. */
      imports: string[];
      /** CSS chunks (Phase 1.5f). */
      css: string[];
    }
  >;
  /** Self-hosted fonts (next/font parity): structured `@font-face`s + the
   *  `:root` CSS variables + the woff2 files to preload. Global (route-
   *  independent); rendered into every SSR `<head>` against `public_prefix`.
   *  Absent when the app declares no `font({...})`. */
  fonts?: ManifestFonts;
}

/** Result of an in-process build — same shape the protocol returns. */
export interface BuildOutput {
  manifestPath: string;
  outdir: string;
}

/**
 * Single-flight in-process build promise. SSR + asset-route handlers
 * both reach for `buildClientBundle()` lazily, so without dedup we
 * could fire two concurrent Bun.build calls under load and trample
 * each other's outputs (especially the `rm -rf outdir` step). The
 * Promise is kept as long as a build is in flight, then cleared so
 * the next invalidation re-builds.
 */
let _inflightBuild: Promise<BuildOutput> | null = null;

/**
 * If a complete prebuilt client-build is present on disk (Pylon Cloud: shipped
 * in the artifact by the builder, marked with `.prebuilt` written last), return
 * its manifest + outdir so callers skip the rebuild entirely. Returns null when
 * there's no prebuilt bundle (dev, or a deploy that didn't pre-build) — callers
 * then build normally.
 */
async function _prebuiltBundle(): Promise<BuildOutput | null> {
  const fsMod: any = await import("node:fs");
  const pathMod: any = await import("node:path");
  const fs = fsMod.default ?? fsMod;
  const path = pathMod.default ?? pathMod;
  const outdir = path.join(process.cwd(), ".pylon", "client-build");
  const manifestPath = path.join(outdir, "manifest.json");
  if (!fs.existsSync(manifestPath)) return null;

  // Reuse a prebuilt bundle when it's explicitly marked (`.prebuilt`) OR when
  // its manifest already targets an ABSOLUTE (CDN) `public_prefix`. The builder
  // bakes an absolute prefix only when it pre-built for the CDN, and it
  // published the hashed assets under THOSE exact hashes — so the runtime must
  // serve this exact manifest verbatim (a local rebuild emits different hashes
  // that would 404 on the CDN). The manifest is a regular file that always
  // ships with the build; keying on it (not just the `.prebuilt` dotfile, which
  // can be dropped in transit) makes reuse robust. A same-origin
  // `/_pylon/build/` manifest is a normal dev/local build → don't short-circuit
  // (let dev hot-rebuild).
  const marker = path.join(outdir, ".prebuilt");
  if (fs.existsSync(marker)) return { manifestPath, outdir };
  try {
    const m = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    if (
      typeof m.public_prefix === "string" &&
      /^https?:\/\//i.test(m.public_prefix)
    ) {
      return { manifestPath, outdir };
    }
  } catch {
    /* unreadable/partial manifest → fall through and rebuild */
  }
  return null;
}

/**
 * Run the bundler in-process and return the manifest path + outdir.
 * Used from `handleBundleClient` (protocol RPC path from Rust) AND
 * from `getManifest` (in-process SSR path).
 */
export async function buildClientBundle(
  appDirRel: string = "app",
): Promise<BuildOutput> {
  // Prebuilt bundle (Pylon Cloud): the builder already produced a complete,
  // content-hashed client-build with `public_prefix` baked to the CDN, shipped
  // it in the artifact, and published the hashed assets to the CDN. Reuse it
  // verbatim instead of rebuilding on the app machine — rebuilding would (a)
  // burn cold-start CPU on every machine on every boot and (b) risk emitting a
  // manifest whose hashes don't match the assets already on the CDN. The
  // `.prebuilt` marker is written LAST by the builder, so a torn/partial copy
  // is never mistaken for complete. Dev never writes the marker, so local
  // hot-rebuild is unaffected.
  const prebuilt = await _prebuiltBundle();
  if (prebuilt) return prebuilt;

  if (_inflightBuild) return _inflightBuild;
  _inflightBuild = (async () => {
    try {
      return await _doBuild(appDirRel);
    } finally {
      _inflightBuild = null;
    }
  })();
  return _inflightBuild;
}

async function _doBuild(appDirRel: string): Promise<BuildOutput> {
  // node:* are available in Bun, but `globalThis.require` is
  // not defined in ESM. Use dynamic import; Bun fast-paths these.
  const fsMod: any = await import("node:fs");
  const pathMod: any = await import("node:path");
  const fs = fsMod.default ?? fsMod;
  const path = pathMod.default ?? pathMod;
  const cwd = process.cwd();
  return _doBuildInner(fs, path, cwd, appDirRel);
}

/**
 * Compile `app/globals.css` through Tailwind v4 (`@tailwindcss/cli`)
 * if both are present. Returns the relative output path (under
 * outdir) when produced, else null. Skipped silently when the
 * project hasn't opted in to Tailwind — we don't want every SSR
 * project to need Tailwind installed.
 */
async function buildTailwind(
  fs: any,
  path: any,
  cwd: string,
  outdir: string,
  appDirRel: string,
): Promise<string | null> {
  const globalsPath = path.join(cwd, appDirRel, "globals.css");
  if (!fs.existsSync(globalsPath)) return null;
  // Resolve @tailwindcss/cli. The package only exports
  // `./package.json` in its `exports` map (it's a binary, not a
  // library), so we resolve THAT and reach for `dist/index.mjs`
  // next to it. If the dep isn't installed, surface a clear hint.
  let cliPath: string;
  try {
    const pkgPath = (Bun as any).resolveSync(
      "@tailwindcss/cli/package.json",
      cwd,
    );
    cliPath = path.join(path.dirname(pkgPath), "dist", "index.mjs");
    if (!fs.existsSync(cliPath)) {
      throw new Error(`tailwindcss CLI entry not found at ${cliPath}`);
    }
  } catch (err: any) {
    throw new Error(
      `app/globals.css exists but @tailwindcss/cli is not installed — run \`bun add @tailwindcss/cli tailwindcss\` (resolver said: ${err?.message ?? err})`,
    );
  }

  // Compile to a temp file first, then name the asset by a hash of the
  // COMPILED OUTPUT — NOT the source. Hashing `globals.css` was the wrong
  // input: adding a Tailwind class in any component regenerates the CSS but
  // leaves globals.css untouched, so the `styles-<hash>.css` filename never
  // changed and browsers / CDNs kept serving the STALE stylesheet (missing the
  // new classes — dropdowns rendered unstyled, etc.). The compiled output, by
  // contrast, changes iff a scanned class changes: its hash busts the cache
  // exactly when it must, and stays identical (cache stays warm) otherwise.
  //
  // Spawn the CLI. Bun is already running; reuse it as the interpreter so the
  // user doesn't need node on PATH.
  const tmpPath = path.join(outdir, ".styles.build.css");
  const proc = (Bun as any).spawn({
    cmd: [process.execPath, cliPath, "-i", globalsPath, "-o", tmpPath, "--minify"],
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  const exitCode = await proc.exited;
  if (exitCode !== 0) {
    const err = await new Response(proc.stderr).text();
    throw new Error(`tailwindcss build failed (exit ${exitCode}): ${err}`);
  }

  const out = fs.readFileSync(tmpPath, "utf8");
  let hash = 0;
  for (let i = 0; i < out.length; i++) {
    hash = (hash * 31 + out.charCodeAt(i)) >>> 0;
  }
  // Pad the base36 hash to 8 chars: the runtime's `is_hashed_name`
  // (frontend.rs) only sends `Cache-Control: immutable` for hashes ≥8 chars
  // (Bun's JS chunk convention). A 32-bit base36 hash is ≤7 chars, so without
  // the pad the content-hashed CSS would be served `no-cache` — browsers + any
  // CDN would refetch it on every page load (and, behind Cloudflare, wake an
  // autostopped origin).
  const stylesName = `styles-${hash.toString(36).padStart(8, "0")}.css`;
  const outPath = path.join(outdir, stylesName);

  // Drop previous styles-*.css so the outdir doesn't accumulate stale builds
  // (the filename now changes on every class change), then move the freshly
  // compiled file into place.
  try {
    for (const f of fs.readdirSync(outdir) as string[]) {
      if (f.startsWith("styles-") && f.endsWith(".css")) {
        fs.rmSync(path.join(outdir, f), { force: true });
      }
    }
  } catch {
    /* outdir may not be listable on the first build — ignore */
  }
  fs.renameSync(tmpPath, outPath);
  return stylesName;
}

async function _doBuildInner(
  fs: any,
  path: any,
  cwd: string,
  appDirRel: string,
): Promise<BuildOutput> {
  const routes = discoverRoutes(fs, path, cwd, appDirRel);
    if (routes.length === 0) {
      throw new Error(
        `no SSR routes discovered under ${appDirRel}/ — nothing to bundle`,
      );
    }

    const stageDir = path.join(cwd, ".pylon");
    fs.mkdirSync(stageDir, { recursive: true });

    // Wipe stale per-route entries so deletions are picked up.
    // Hashed chunk outputs live under client-build/ and are wiped
    // by the same `rm -rf` so renamed routes don't leave orphans.
    for (const name of fs.readdirSync(stageDir)) {
      if (name.startsWith("client-entry-")) {
        try {
          fs.unlinkSync(path.join(stageDir, name));
        } catch {
          /* ignore */
        }
      }
    }
    const outdir = path.join(stageDir, "client-build");
    try {
      fs.rmSync(outdir, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
    fs.mkdirSync(outdir, { recursive: true });

    // The shared runtime + per-route entries. We track which file
    // each entry corresponds to so we can match metafile.outputs
    // back to the original route afterwards.
    const runtimePath = path.join(stageDir, "client-runtime.ts");
    fs.writeFileSync(runtimePath, CLIENT_RUNTIME_SOURCE, "utf8");

    // Emit the not-found/error boundary as a sibling module the runtime
    // imports (`./client-boundary`). Its source is this package's real
    // ssr-client-boundary.ts — ONE source of truth, unit-tested directly,
    // rather than an inline string in CLIENT_RUNTIME_SOURCE that nothing
    // could render. Bun pulls it into the shared chunk via the runtime's
    // static `import { createPylonBoundary } from "./client-boundary"`.
    //
    // Resolve THIS module's directory from the standard `import.meta.url`
    // (via node:url) rather than Bun's `import.meta.dir` — the latter is a
    // Bun-only extension that `tsc` doesn't know about, so any app that
    // imports `@pylonsync/functions` and runs `tsc` would otherwise fail
    // type-checking on this file. `import.meta.url` works in Bun and Node.
    const urlMod: any = await import("node:url");
    const fileURLToPath = urlMod.fileURLToPath ?? urlMod.default?.fileURLToPath;
    const here = path.dirname(fileURLToPath(import.meta.url));
    fs.writeFileSync(
      path.join(stageDir, "client-boundary.ts"),
      fs.readFileSync(path.join(here, "ssr-client-boundary.ts"), "utf8"),
      "utf8",
    );

    const entryPaths: string[] = [];
    // entryPath (absolute) → component path (for manifest lookup).
    const entryToComponent = new Map<string, string>();
    for (const r of routes) {
      const slug = slugForComponent(r.component);
      const entryPath = path.join(stageDir, `client-entry-${slug}.tsx`);
      fs.writeFileSync(entryPath, generateRouteEntry(r), "utf8");
      entryPaths.push(entryPath);
      entryToComponent.set(entryPath, r.component);
    }

    // splitting: true gates code-splitting. Bun 1.3.14 does NOT
    // expose a `metafile` flag — its build result lists per-output
    // `path` + `kind` (`entry-point` vs `chunk`) but no import
    // graph. We recover the per-entry preload set by parsing the
    // entry files' literal `import "./chunks/<name>.js"`
    // statements after the build.
    const result = await Bun.build({
      entrypoints: entryPaths,
      outdir,
      // Pin the build root to the project dir, NOT the inferred common
      // parent of the entrypoints (which is `.pylon/`). Bun discovers
      // `tsconfig.json` from `root`, and the entries we stage under
      // `.pylon/client-entry-*.tsx` would otherwise resolve tsconfig from
      // `.pylon/` and never see the project's `compilerOptions.paths` — so
      // `@/` path-alias imports (e.g. shadcn's `@/lib/utils`) compile fine
      // server-side but fail in the client bundle. `[name]-[hash]` naming
      // has no `[dir]`, so pinning root doesn't move outputs. Keeps client
      // and server alias resolution identical.
      root: cwd,
      target: "browser",
      format: "esm",
      minify: true,
      sourcemap: "none",
      splitting: true,
      // The browser has no `process`. React, next-themes, sonner, and most
      // npm UI deps reference `process.env.NODE_ENV` (and migrated Next code
      // may reference other `process.env.*`), so without this the very first
      // such reference throws `ReferenceError: process is not defined` during
      // hydration — React then unmounts the tree and the page renders blank.
      // Statically replace `process.env.NODE_ENV` with the build mode and
      // collapse any other `process.env.*` to an empty object (→ undefined),
      // so no `process` reference survives into the browser bundle.
      define: {
        "process.env.NODE_ENV": JSON.stringify(
          process.env.NODE_ENV === "development" ? "development" : "production",
        ),
        "process.env": "({})",
      },
      naming: {
        entry: "[name]-[hash].js",
        chunk: "chunks/[name]-[hash].js",
        asset: "assets/[name]-[hash][ext]",
      },
    });

    if (!result.success) {
      const msgs = (result.logs ?? [])
        .map((l) => `${l.level}: ${l.message}`)
        .join("\n");
      throw new Error(`Bun.build failed:\n${msgs || "(no log messages)"}`);
    }

    // Index outputs:
    //   - entries (kind === "entry-point") — matched to components
    //     by filename stem (`client-entry-<slug>`).
    //   - chunks (kind === "chunk") — looked up by basename when
    //     scanning entry files for static `import "./chunks/..."`
    //     specifiers.
    const outdirRel = path.relative(cwd, outdir);
    const entriesByStem = new Map<
      string,
      { absPath: string; relPath: string }
    >();
    const chunksByBasename = new Map<
      string,
      { absPath: string; relPath: string }
    >();
    for (const o of result.outputs) {
      const absPath: string = o.path;
      const relPath = path.relative(outdir, absPath);
      const base = path.basename(absPath);
      // Strip `-<hash>.js` to recover the entry source's stem
      // (e.g. `client-entry-app__hello__page`). The hash is
      // alphanumeric (Bun uses base36-ish), the slug we wrote is
      // `[A-Za-z0-9_]+`, so splitting on the LAST `-<hash>.js` is
      // unambiguous.
      const stem = base.replace(/-[A-Za-z0-9]+\.(?:m?js)$/, "");
      if (o.kind === "entry-point") {
        entriesByStem.set(stem, { absPath, relPath });
      } else if (o.kind === "chunk") {
        chunksByBasename.set(base, { absPath, relPath });
      }
    }

    // Scan a built JS file for static `import` literals pointing
    // at `./chunks/<file>.js` and return them resolved to outdir-
    // relative paths. Bun's minified output uses simple double
    // quotes for module specifiers, so a `matchAll` covers both
    // `import X from "Y"` and bare `import "Y"`.
    function scanChunkImports(jsAbsPath: string): string[] {
      let src: string;
      try {
        src = fs.readFileSync(jsAbsPath, "utf8");
      } catch {
        return [];
      }
      const found = new Set<string>();
      const matches = src.matchAll(
        /(?:from\s*|import\s*\(?\s*)["']([^"']+)["']/g,
      );
      for (const m of matches) {
        const spec = m[1];
        if (spec.startsWith("./chunks/") || spec.startsWith("chunks/")) {
          const base = path.basename(spec);
          const hit = chunksByBasename.get(base);
          if (hit) found.add(hit.relPath);
        }
      }
      return Array.from(found);
    }

    const manifest: PylonBundleManifest = {
      build_id: makeBuildId(),
      outdir: outdirRel,
      public_prefix: "/_pylon/build/",
      routes: {},
    };
    for (const r of routes) {
      const slug = slugForComponent(r.component);
      const stem = `client-entry-${slug}`;
      const entry = entriesByStem.get(stem);
      if (!entry) continue;
      // Walk transitively in case Bun emits chunks that reference
      // other chunks (rare in the single-level splitting we use,
      // but free to compute).
      const seen = new Set<string>();
      const queue: string[] = scanChunkImports(entry.absPath);
      for (const q of queue) seen.add(q);
      while (queue.length > 0) {
        const relChunk = queue.shift()!;
        const absChunk = path.join(outdir, relChunk);
        for (const nested of scanChunkImports(absChunk)) {
          if (!seen.has(nested)) {
            seen.add(nested);
            queue.push(nested);
          }
        }
      }
      manifest.routes[r.component] = {
        file: entry.relPath,
        imports: Array.from(seen),
        css: [],
      };
    }

    // Bail loudly if discovery succeeded but the manifest came
    // out empty — means our entryPoint → component matching broke
    // and SSR will silently hydration-skip.
    if (Object.keys(manifest.routes).length === 0) {
      throw new Error(
        "manifest is empty after build — entryPoint matching against metafile failed",
      );
    }
    if (Object.keys(manifest.routes).length !== routes.length) {
      const missing = routes
        .filter((r) => !(r.component in manifest.routes))
        .map((r) => r.component);
      throw new Error(
        `manifest missing entries for routes: ${missing.join(", ")}`,
      );
    }

    // Tailwind v4 compile. Optional — only fires if the project has
    // `app/globals.css`. Adds the stylesheet to every route's css
    // array so SSR head injection emits `<link rel="stylesheet">`.
    try {
      const styles = await buildTailwind(fs, path, cwd, outdir, appDirRel);
      if (styles) {
        for (const r of Object.values(manifest.routes)) {
          r.css = [styles];
        }
      }
    } catch (twErr: any) {
      // Tailwind failure shouldn't kill the SSR build — log a loud
      // warning + ship the bundle without styles so devs can iterate.
      // eslint-disable-next-line no-console
      console.warn(`[pylon ssr] tailwind compile failed: ${twErr?.message ?? twErr}`);
    }

    // Self-hosted fonts (next/font parity). Reads `fonts` from the app's
    // pylon.manifest.json, fetches + self-hosts each woff2 into outdir (served
    // under /_pylon/build/), and bakes the structured faces + size-adjusted
    // fallback metrics into the bundle manifest for SSR head injection. On
    // Pylon Cloud the builder runs this same path, so the woff2 + faces ship in
    // the prebuilt artifact. A fetch/parse failure degrades to a variable-only
    // entry — it never kills the build.
    try {
      const declaredFonts = readManifestFonts(fs, path, cwd);
      if (declaredFonts.length > 0) {
        const builtFonts = await buildFonts(fs, path, cwd, outdir, declaredFonts);
        if (builtFonts) manifest.fonts = builtFonts;
      }
    } catch (fErr: any) {
      // eslint-disable-next-line no-console
      console.warn(`[pylon ssr] font build failed: ${fErr?.message ?? fErr}`);
    }

  const manifestPath = path.join(outdir, "manifest.json");
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2), "utf8");

  // Bump our in-process manifest cache so SSR re-reads on next request.
  _manifestCache = null;

  return { manifestPath, outdir };
}

/**
 * Cached parsed manifest for the SSR head-injection path. Keyed on
 * mtime so an external `pylon build` that overwrites manifest.json
 * gets picked up by the next SSR request without a process restart.
 */
let _manifestCache: { mtimeMs: number; data: PylonBundleManifest } | null = null;

/**
 * Return the bundle manifest. If a fresh manifest exists on disk,
 * use it (caching parse output across requests). Otherwise build
 * the bundle in-process (deduped by `buildClientBundle`) and read.
 *
 * Called from `ssr-runtime.ts` per-request, so the disk-stat fast
 * path matters. Bun's `fs.statSync` is a ~5µs syscall; cheap enough
 * that we don't gate it on a flag.
 */
export async function getManifest(): Promise<PylonBundleManifest> {
  const fsMod: any = await import("node:fs");
  const pathMod: any = await import("node:path");
  const fs = fsMod.default ?? fsMod;
  const path = pathMod.default ?? pathMod;
  const cwd = process.cwd();
  const manifestPath = path.join(cwd, ".pylon", "client-build", "manifest.json");

  if (fs.existsSync(manifestPath)) {
    const stat = fs.statSync(manifestPath);
    if (_manifestCache && _manifestCache.mtimeMs === stat.mtimeMs) {
      return _manifestCache.data;
    }
    const raw = fs.readFileSync(manifestPath, "utf8");
    const data = JSON.parse(raw) as PylonBundleManifest;
    _manifestCache = { mtimeMs: stat.mtimeMs, data };
    return data;
  }

  // Manifest missing → build in-process, then read.
  const { manifestPath: built } = await buildClientBundle();
  const raw = fs.readFileSync(built, "utf8");
  const data = JSON.parse(raw) as PylonBundleManifest;
  const stat = fs.statSync(built);
  _manifestCache = { mtimeMs: stat.mtimeMs, data };
  return data;
}

/**
 * Protocol entry. Builds + responds. Rust calls this via the
 * `bundle_client` RPC; on success the response carries both
 * the manifest path (so Rust can load it if it wants, today it
 * doesn't) and the outdir (so `/_pylon/build/<rel>` serves the
 * right tree).
 */
export async function handleBundleClient(
  msg: BundleClientMessage,
  send: Send,
): Promise<void> {
  try {
    const appDirRel =
      msg.app_dir && msg.app_dir.length > 0 ? msg.app_dir : "app";
    const { manifestPath, outdir } = await buildClientBundle(appDirRel);
    send({
      type: "bundle_client_result",
      call_id: msg.call_id,
      path: manifestPath,
      outdir,
    });
  } catch (err: any) {
    send({
      type: "bundle_client_result",
      call_id: msg.call_id,
      path: "",
      outdir: "",
      error: err?.message || String(err),
    });
  }
}

/**
 * Stable-ish build id. We don't have Date.now() in workflow scripts
 * but Bun's runtime is fine — performance.now() + a counter would
 * also do. Falling back to a randomish hex string keyed on the
 * process pid + a monotonic counter is good enough for telling
 * "did the bundle change" without claiming to be cryptographic.
 */
let _buildCounter = 0;
function makeBuildId(): string {
  _buildCounter += 1;
  return `${process.pid.toString(36)}-${Date.now().toString(36)}-${_buildCounter}`;
}

/**
 * Base URL/path prepended to every SSR-emitted asset reference (the entry
 * `<script type="module">`, `<link rel="stylesheet">`, and `modulepreload`s)
 * and to client-side chunk loads — baked into the bundle manifest as
 * `public_prefix`.
 *
 * Defaults to `/_pylon/build/`, which the Rust host serves from the build's
 * local `outdir`. That keeps dev and any deploy that doesn't opt in unchanged.
 *
 * Set `PYLON_PUBLIC_PREFIX` to an ABSOLUTE base (a CDN / object-storage URL for
 * this build's content-hashed assets, e.g. `https://assets.pyln.dev/<build>/`)
 * to serve assets off the app machines entirely. This is what makes hashed
 * assets survive an artifact prune and resolve IDENTICALLY across every Fly
 * machine: the entry filename SSR emits becomes an absolute URL that always
 * exists, instead of a machine-local path a prune/rollover can 404 (the bug
 * where a stale machine emitted `client-entry-…-<oldhash>.js` after that file
 * was pruned, breaking hydration). A trailing slash is enforced so
 * `${prefix}${file}` always joins correctly.
 */
function resolvePublicPrefix(): string {
  const raw = (process.env.PYLON_PUBLIC_PREFIX ?? "").trim();
  if (!raw) return "/_pylon/build/";
  return raw.endsWith("/") ? raw : `${raw}/`;
}
