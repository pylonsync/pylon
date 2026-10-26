// The browser half of ssr-client-nav-runtime.test.ts, run in its own Bun
// process: it stages CLIENT_RUNTIME_SOURCE and its sibling modules exactly as
// the bundler does (writeClientRuntimeModules), imports the runtime into a
// happy-dom window, hydrates a page, taps links, goes back and forward, and
// prints what it saw as JSON. Its own process, because loading react-dom and
// the staged runtime into the test runner broke later Bun.build calls in the
// same run (EISDIR reading scheduler/index.js).

import fs from "node:fs";
import path from "node:path";
import { Window } from "happy-dom";
import React from "react";
import { renderToString } from "react-dom/server";
import { writeClientRuntimeModules } from "./ssr-client-bundler";

const PKG_NODE_MODULES = path.join(import.meta.dir, "..", "node_modules");

const MANIFEST = {
  build_id: "t",
  outdir: ".pylon/client-build",
  public_prefix: "/_pylon/build/",
  routes: {
    "app/home/page": { file: "home.js", imports: [], css: [], path: "/" },
    "app/fast/page": { file: "fast.js", imports: [], css: [], path: "/fast", navigation: "client" },
    "app/slow/page": { file: "slow.js", imports: [], css: [], path: "/slow" },
    "app/offline/page": { file: "offline.js", imports: [], css: [], path: "/offline" },
    // Never registered by a test hydrate() call, and their files do not
    // exist: loading them fails like a file the deploy removed.
    "app/reports/page": { file: "reports-v1.js", imports: [], css: [], path: "/reports" },
    "app/quick/page": { file: "quick-v1.js", imports: [], css: [], path: "/quick", navigation: "client" },
  },
  offline: { routes: ["/offline"] },
};

/** \`stale-manifest\`: the page runs build v1, and the build manifest it gets
 *  is build v0's, as an older service worker answers from its cache. */
const MODE = process.argv[2] ?? "default";

/** Build v0's manifest: other files for the same routes. */
const STALE_MANIFEST = {
  ...MANIFEST,
  routes: Object.fromEntries(
    Object.entries(MANIFEST.routes).map(([k, v]) => [k, { ...v, file: v.file.replace(/(-v1)?\.js$/, "-v0.js") }]),
  ),
  offline: { ...MANIFEST.offline, version: "v0" },
};

/** The page's build: the home page's __PYLON_DATA__ carries it. */
const PAGE_BUILD = "v1";
/** The build the server runs after the deploy. */
let serverBuild = "v1";

const LINKS = ["/", "/fast", "/slow", "/offline", "/reports", "/quick"];

function Layout({ children }: { children?: React.ReactNode }) {
  return React.createElement(
    "html",
    null,
    React.createElement("head", null, React.createElement("title", null, "App")),
    React.createElement(
      "body",
      null,
      React.createElement(
        "nav",
        null,
        ...LINKS.map((href) =>
          React.createElement(
            "a",
            { key: href, href, "data-pylon-link": "", id: "to" + (href.slice(1) || "home") },
            href,
          ),
        ),
      ),
      React.createElement("main", { id: "page" }, children),
    ),
  );
}

const page = (name: string) =>
  function Page(props: any) {
    return React.createElement("h1", null, `${name} ${props.pathname ?? ""}`);
  };
const Home = page("home");
const Fast = page("fast");
const Slow = page("slow");
const Offline = page("offline");

const fetches: string[] = [];
const scrolls: number[] = [];
/** Full page loads (\`location.href = ...\`) the runtime started. */
const fullLoads: string[] = [];
/** Calls to @pylonsync/react's moveToLatestBuild, and its answer. */
const moves: any[] = [];
let moveResult = "moving";
/** The server's build as the page's updater last saw it: a recheck asks
 *  the server; null until then (no check has run). */
let knownBuild: string | null = null;
let win: any;
let rt: any;
let dir = "";

function pylonData(component: string, pathname: string, build = serverBuild): string {
  const data = {
    component,
    layouts: [],
    props: { pathname, url: pathname, params: {}, searchParams: {} },
    ssrData: {},
    offline: { version: build },
  };
  return `<script id="__PYLON_DATA__" type="application/json">${JSON.stringify(data)}</script>`;
}

async function settle() {
  for (let i = 0; i < 20; i++) await new Promise((r) => setTimeout(r, 0));
}

function click(id: string) {
  const el = win.document.getElementById(id);
  el.dispatchEvent(new win.MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
}

// A browser fires popstate after back() and forward(); happy-dom 15 moves
// the location without it, so the test fires it.
async function traverse(direction: "back" | "forward") {
  win.history[direction]();
  win.dispatchEvent(new win.PopStateEvent("popstate", { state: win.history.state }));
  await settle();
}

const heading = () => win.document.querySelector("#page h1")?.textContent;
const pageFetches = () => fetches.filter((u) => !u.includes("/_pylon/build/manifest.json"));

async function setUp() {
  // Inside this package's node_modules, so the runtime resolves react and
  // react-dom the way the test does. A symlinked node_modules in a temp dir
  // left Bun's resolver cache broken for later Bun.build calls in the run.
  const cacheDir = path.join(PKG_NODE_MODULES, ".cache");
  fs.mkdirSync(cacheDir, { recursive: true });
  dir = fs.mkdtempSync(path.join(cacheDir, "pylon-client-nav-"));
  await writeClientRuntimeModules(fs, path, dir);
  // @pylonsync/react is not a dependency of this package: stand in for the
  // functions the runtime calls.
  const runtimeFile = path.join(dir, "client-runtime.ts");
  fs.writeFileSync(
    runtimeFile,
    fs
      .readFileSync(runtimeFile, "utf8")
      .replaceAll('from "@pylonsync/react"', 'from "./pylon-react-stub"'),
  );
  fs.writeFileSync(
    path.join(dir, "pylon-react-stub.ts"),
    "export function configureClient() {}\n" +
      "export async function registerOfflineWorker() { return null; }\n" +
      "export function moveToLatestBuild(options) { return globalThis.__moveToLatestBuild(options); }\n",
  );
  // Answers like AppUpdater.moveToLatestBuild: from the last check, or
  // from the server on a recheck.
  (globalThis as any).__moveToLatestBuild = async (options: any) => {
    moves.push(options);
    if (options.recheck) knownBuild = serverBuild;
    if (knownBuild === null) return "offline";
    if (knownBuild === PAGE_BUILD) return "current";
    return moveResult;
  };
  fs.writeFileSync(path.join(dir, "loading-registry.ts"), "export const LOADING_MODULES = {};\n");

  win = new Window({ url: "http://localhost/" });
  const g = globalThis as any;
  g.window = win;
  g.document = win.document;
  g.navigator = win.navigator;
  g.location = win.location;
  g.history = win.history;
  g.HTMLElement = win.HTMLElement;
  g.Node = win.Node;
  g.Event = win.Event;
  g.MouseEvent = win.MouseEvent;
  g.DOMParser = win.DOMParser;
  g.MutationObserver = win.MutationObserver;
  g.requestAnimationFrame = (cb: any) => setTimeout(() => cb(Date.now()), 0);
  g.cancelAnimationFrame = (id: any) => clearTimeout(id);
  win.scrollTo = (_x: number, y: number) => scrolls.push(y);
  // Record full page loads instead of letting happy-dom navigate.
  const hrefDescriptor = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(win.location),
    "href",
  )!;
  Object.defineProperty(win.location, "href", {
    configurable: true,
    get: () => hrefDescriptor.get!.call(win.location),
    set: (value: string) => fullLoads.push(value),
  });
  const fakeFetch = async (input: any) => {
    const url = typeof input === "string" ? input : input.url;
    fetches.push(url);
    if (url === "/_pylon/build/manifest.json") {
      const body = MODE === "stale-manifest" ? STALE_MANIFEST : MANIFEST;
      return new Response(JSON.stringify(body), { status: 200 });
    }
    const pathname = new URL(url, "http://localhost").pathname;
    if (pathname === "/slow") {
      return new Response(
        `<!DOCTYPE html><html><head><title>Slow</title></head><body>${pylonData("app/slow/page", pathname)}</body></html>`,
        { status: 200 },
      );
    }
    if (pathname === "/reports") {
      return new Response(
        `<!DOCTYPE html><html><head><title>Reports</title></head><body>${pylonData("app/reports/page", pathname)}</body></html>`,
        { status: 200 },
      );
    }
    return new Response("not found", { status: 404 });
  };
  g.fetch = fakeFetch;
  win.fetch = fakeFetch;

  // The server-rendered home page, with its hydration data.
  const props = { pathname: "/", url: "/", params: {}, searchParams: {} };
  const html = renderToString(React.createElement(Layout, null, React.createElement(Home, props)));
  const parsed = new win.DOMParser().parseFromString(
    "<!DOCTYPE html>" + html.replace("</body>", pylonData("app/home/page", "/", PAGE_BUILD) + "</body>"),
    "text/html",
  );
  // Remove, then append: happy-dom 20's replaceChild inserts the new root
  // before it removes the old one, which a document refuses.
  const root = win.document.importNode(parsed.documentElement, true);
  win.document.documentElement.remove();
  win.document.appendChild(root);

  rt = await import(path.join(dir, "client-runtime.ts"));
  rt.hydrate("app/home/page", Home, [Layout]);
  await settle();
  // The other routes' entries, as their chunks register them.
  rt.hydrate("app/fast/page", Fast, [Layout], { title: "Fast" });
  rt.hydrate("app/slow/page", Slow, [Layout]);
  rt.hydrate("app/offline/page", Offline, [Layout]);
}




interface Observation {
  heading: string | null;
  pathname: string;
  pageFetches: string[];
  historyLength: number;
  scrolls: number[];
  title: string;
}

function observe(): Observation {
  const seen = {
    heading: heading() ?? null,
    pathname: win.location.pathname,
    pageFetches: pageFetches(),
    historyLength: win.history.length,
    scrolls: [...scrolls],
    title: win.document.title,
  };
  fetches.length = 0;
  scrolls.length = 0;
  return seen;
}

const report: Record<string, any> = {};

const preloads = () =>
  [...win.document.querySelectorAll('link[rel="modulepreload"]')].map((l: any) => l.getAttribute("href"));

/** Prefetch timing: chunks for a link on screen wait for idle and skip
 *  Save-Data; intent fetches at once. */
async function prefetchTiming() {
  const pylon = win.__pylon;
  const before = preloads().length;
  // Idle callbacks run only when the test says so.
  const idle: Array<() => void> = [];
  win.requestIdleCallback = (fn: () => void) => {
    idle.push(fn);
    return idle.length;
  };
  await pylon.prefetch("/fast");
  await settle();
  const sightBeforeIdle = preloads().length - before;
  for (const fn of idle.splice(0)) fn();
  await settle();
  const sightAfterIdle = preloads().length - before;

  // Save-Data: nothing on sight, even when idle.
  Object.defineProperty(win.navigator, "connection", {
    value: { saveData: true, effectiveType: "4g" },
    configurable: true,
  });
  const beforeSave = preloads().length;
  await pylon.prefetch("/offline");
  await settle();
  for (const fn of idle.splice(0)) fn();
  await settle();
  const saveDataSight = preloads().length - beforeSave;

  // Intent fetches at once, Save-Data or not.
  await pylon.prefetch("/offline", { document: true });
  await settle();
  const intent = preloads().length - beforeSave;
  return { sightBeforeIdle, sightAfterIdle, saveDataSight, intent };
}
/** A deploy while the page is open: its build's route files are gone. */
async function afterDeploy() {
  const out: Record<string, any> = {};
  const take = () => {
    const seen = {
      moves: moves.splice(0),
      fullLoads: fullLoads.splice(0),
      heading: heading() ?? null,
      pageFetches: pageFetches(),
    };
    fetches.length = 0;
    return seen;
  };
  take();

  // Before the deploy, a route entry that fails to load (a bug, not a
  // deploy) still falls back to one full page load.
  click("toreports");
  await settle();
  out.sameBuildFailure = take();

  serverBuild = "v2";
  knownBuild = null;
  // A click on a page route: the server answers with build v2's page. The
  // page moves to v2 once, through moveToLatestBuild, opening the link;
  // before the fix this was a full load of the old build, on every click.
  click("toreports");
  await settle();
  out.pageRoute = take();

  // A click on a route that renders on the client, whose entry the deploy
  // removed: the same move, with no page fetch.
  knownBuild = null;
  click("toquick");
  await settle();
  out.clientRoute = take();

  // The page's last check already saw the new build: the click moves at
  // once, with no page fetch and no entry load.
  knownBuild = "v2";
  click("toreports");
  await settle();
  out.knownAhead = take();

  // Offline: the entry cannot load and the server cannot be asked. The
  // page keeps its screen; no full load, which would show a kept page.
  moveResult = "offline";
  knownBuild = null;
  serverBuild = null as any;
  click("toquick");
  await settle();
  out.offline = take();

  // A page that already moved and is still on the old build: one full
  // load, and no loop of moves.
  moveResult = "blocked";
  serverBuild = "v2";
  knownBuild = null;
  click("toreports");
  await settle();
  out.blocked = take();
  return out;
}

/** The page of build v1 got build v0's manifest. Before the fix it loaded
 *  v0's route files (gone, so every click was a full page load). */
async function staleManifest() {
  const warnings: string[] = [];
  const warn = console.warn;
  console.warn = (...args: any[]) => warnings.push(args.map(String).join(" "));
  try {
    await setUp();
    // The server and the page agree on v1; the page's own worker waits
    // behind the old one, so the updater moves.
    knownBuild = null;
    moveResult = "moving";
    serverBuild = "v1";
    (globalThis as any).__moveToLatestBuild = async (options: any) => {
      moves.push(options);
      return options.recheck ? "moving" : "offline";
    };
    click("toreports");
    await settle();
    return {
      moves: moves.splice(0),
      fullLoads: fullLoads.splice(0),
      preloads: preloads(),
      pageFetches: pageFetches(),
      warnedAboutManifest: warnings.some((w) => w.includes("build manifest is for build v0")),
    };
  } finally {
    console.warn = warn;
  }
}

async function defaultFlow() {
  await setUp();
  report.hydrated = observe();
  click("tofast");
  await settle();
  report.fast = observe();
  click("tooffline");
  await settle();
  report.offline = observe();
  click("toslow");
  await settle();
  report.slow = observe();
  await traverse("back");
  report.back1 = observe();
  await traverse("back");
  report.back2 = observe();
  await traverse("forward");
  report.forward1 = observe();
  await traverse("forward");
  report.forward2 = observe();
  report.prefetch = await prefetchTiming();
  report.deploy = await afterDeploy();
}

try {
  if (MODE === "stale-manifest") report.staleManifest = await staleManifest();
  else await defaultFlow();
} finally {
  if (dir) fs.rmSync(dir, { recursive: true, force: true });
}
console.log("REPORT " + JSON.stringify(report));
process.exit(0);
