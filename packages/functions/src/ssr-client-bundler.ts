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

type Send = (msg: Record<string, unknown>) => void;

interface BundleClientMessage {
  type: "bundle_client";
  call_id: string;
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
 * Synchronously walk `app/` under cwd. Returns one entry per
 * discovered page, each carrying its layout chain (root → leaf).
 * Mirrors the discovery logic in @pylonsync/sdk's
 * `discoverAppRoutes` exactly — same sort order, same group-strip,
 * so the in-browser map keys line up with the manifest's component
 * field.
 */
function discoverRoutes(
  fs: any,
  path: any,
  cwd: string,
): DiscoveredRoute[] {
  const appDir = path.join(cwd, "app");
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
 * The shared hydration dispatcher. ONE module, imported by every
 * per-route entry. Bun's splitter will see N entries reach for this
 * single module and pull it (and React via the transitive imports)
 * into a shared chunk.
 *
 * We pass the Page + Layouts in by reference instead of via a route
 * map because each entry already knows its own page; there's no
 * runtime dispatch on the component path. The server picked the
 * route entry to load, so by the time this runs we're committed.
 */
const CLIENT_RUNTIME_SOURCE = `// Generated by Pylon SSR (Phase 1.5e shared hydration runtime).
// DO NOT EDIT — overwritten on every pylon dev / build.

import { createElement } from "react";
import { hydrateRoot } from "react-dom/client";

export function hydrate(Page, Layouts) {
  function main() {
    const dataEl = document.getElementById("__PYLON_DATA__");
    if (!dataEl) {
      console.warn("[pylon ssr] __PYLON_DATA__ script tag not found; skipping hydration");
      return;
    }
    let data;
    try {
      data = JSON.parse(dataEl.textContent || "{}");
    } catch (e) {
      console.error("[pylon ssr] failed to parse hydration data:", e);
      return;
    }
    let tree = createElement(Page, data.props);
    for (let i = Layouts.length - 1; i >= 0; i--) {
      const Layout = Layouts[i];
      if (!Layout) continue;
      tree = createElement(Layout, data.props, tree);
    }
    hydrateRoot(document, tree);
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", main);
  } else {
    main();
  }
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
  return `// Generated by Pylon SSR (Phase 1.5e per-route hydration entry).
// DO NOT EDIT — overwritten on every pylon dev / build.

import { hydrate } from "./client-runtime";
import Page from "${cwd_to_import(route.component)}";
${layoutImports}

hydrate(Page, [${layoutArray}]);
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
 * Run the bundler in-process and return the manifest path + outdir.
 * Used from `handleBundleClient` (protocol RPC path from Rust) AND
 * from `getManifest` (in-process SSR path).
 */
export async function buildClientBundle(): Promise<BuildOutput> {
  if (_inflightBuild) return _inflightBuild;
  _inflightBuild = (async () => {
    try {
      return await _doBuild();
    } finally {
      _inflightBuild = null;
    }
  })();
  return _inflightBuild;
}

async function _doBuild(): Promise<BuildOutput> {
  // node:* are available in Bun, but `globalThis.require` is
  // not defined in ESM. Use dynamic import; Bun fast-paths these.
  const fsMod: any = await import("node:fs");
  const pathMod: any = await import("node:path");
  const fs = fsMod.default ?? fsMod;
  const path = pathMod.default ?? pathMod;
  const cwd = process.cwd();
  return _doBuildInner(fs, path, cwd);
}

async function _doBuildInner(fs: any, path: any, cwd: string): Promise<BuildOutput> {
  const routes = discoverRoutes(fs, path, cwd);
    if (routes.length === 0) {
      throw new Error("no SSR routes discovered under app/ — nothing to bundle");
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
      target: "browser",
      format: "esm",
      minify: true,
      sourcemap: "none",
      splitting: true,
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
    const { manifestPath, outdir } = await buildClientBundle();
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
