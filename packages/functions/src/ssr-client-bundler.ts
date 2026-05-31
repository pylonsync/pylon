// Build the client-side hydration bundle for a Pylon SSR project.
//
// Phase 1.5d: discovers `app/**/page.tsx` + `app/**/layout.tsx` under
// the project cwd, writes a synthetic ES module entry that imports
// every page + layout statically, and runs `Bun.build` against it
// targeting the browser. The output lands at
// `.pylon/client.js` under the project cwd; the Rust host reads
// that file from disk and serves it at `/_pylon/client.js`.
//
// The generated entry knows the full discovered route table at
// build time, so the in-browser dispatch is a constant-time map
// lookup keyed on the component path that the SSR'd HTML emits in
// the `__PYLON_DATA__` script tag.
//
// What it doesn't do (Phase 1.5e+):
//   - File-watcher invalidation. Rebuild requires a pylon dev
//     restart.
//   - Code splitting per route. Phase 1.5d ships ONE bundle that
//     includes every page; pages are small enough today that this
//     is fine. Per-route bundles become valuable once apps have
//     dozens of pages.
//   - Source maps. Phase 1.5e enables them for dev-mode debugging.

type Send = (msg: Record<string, unknown>) => void;

interface BundleClientMessage {
  type: "bundle_client";
  call_id: string;
}

interface DiscoveredRoute {
  /** Project-relative module path without extension. */
  component: string;
  /** Layout chain root → leaf. */
  layouts: string[];
}

/** Bun.build returns this shape. */
type BunBuildOutput = {
  success: boolean;
  outputs: Array<{
    path: string;
    kind: string;
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
 * Generate the synthetic client entry. Each discovered page +
 * layout gets a static import bound to a deterministic symbol
 * name. The runtime ROUTES / LAYOUTS maps key on the
 * project-relative module path the server emits in
 * `__PYLON_DATA__.component` / `.layouts`, so dispatch is a hot
 * O(1) lookup.
 *
 * The entry handles the readyState race that hits if the bundle
 * loads BEFORE the inline `__PYLON_DATA__` script tag is parsed
 * (rare with `type="module"` but possible with HTTP/2 push). On
 * load, if the data tag is missing we just return — the next
 * boot will retry.
 */
function generateEntrySource(routes: DiscoveredRoute[]): string {
  // Deterministic symbol name for an arbitrary module path.
  const sym = (p: string, prefix: string): string =>
    `${prefix}_${p.replace(/[^A-Za-z0-9_]/g, "_")}`;

  // Unique layout set across all routes (a layout may be shared by
  // several pages and we only want to import it once).
  const allLayouts = new Set<string>();
  for (const r of routes) for (const l of r.layouts) allLayouts.add(l);

  const pageImports = routes
    .map((r) => `import ${sym(r.component, "P")} from "${cwd_to_import(r.component)}";`)
    .join("\n");
  const layoutImports = Array.from(allLayouts)
    .map((l) => `import ${sym(l, "L")} from "${cwd_to_import(l)}";`)
    .join("\n");

  const routesEntries = routes
    .map(
      (r) =>
        `  ${JSON.stringify(r.component)}: { Page: ${sym(r.component, "P")}, Layouts: [${r.layouts
          .map((l) => sym(l, "L"))
          .join(", ")}] }`,
    )
    .join(",\n");

  return `// Generated by Pylon SSR (Phase 1.5d hydration entry).
// DO NOT EDIT — overwritten on every pylon dev / build.

import { createElement } from "react";
import { hydrateRoot } from "react-dom/client";

${pageImports}
${layoutImports}

const ROUTES = {
${routesEntries}
};

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
  const route = ROUTES[data.component];
  if (!route) {
    console.warn("[pylon ssr] unknown component for hydration:", data.component);
    return;
  }
  let tree = createElement(route.Page, data.props);
  const layouts = data.layouts || [];
  // Walk leaf → root, wrapping with each layout. The server used
  // the same order, so the produced React tree shape matches the
  // server-rendered DOM exactly (hydrate-friendly).
  for (let i = layouts.length - 1; i >= 0; i--) {
    const Layout = route.Layouts[i];
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
`;
}

/**
 * Bun's static analysis of `import "PATH"` resolves paths relative
 * to the SOURCE file at build time. The synthetic entry lives at
 * `<cwd>/.pylon/client-entry.tsx`, so to reach `<cwd>/app/page.tsx`
 * we need to escape one level (`../app/page`). Returns a path
 * suitable to drop into an `import` literal.
 */
function cwd_to_import(modulePath: string): string {
  return `../${modulePath}`;
}

export async function handleBundleClient(
  msg: BundleClientMessage,
  send: Send,
): Promise<void> {
  try {
    // node:* are available in Bun, but `globalThis.require` is
    // not defined in ESM. Use dynamic import; Bun fast-paths these.
    const fsMod: any = await import("node:fs");
    const pathMod: any = await import("node:path");
    const fs = fsMod.default ?? fsMod;
    const path = pathMod.default ?? pathMod;
    const cwd = process.cwd();
    const routes = discoverRoutes(fs, path, cwd);
    if (routes.length === 0) {
      throw new Error("no SSR routes discovered under app/ — nothing to bundle");
    }

    const stageDir = path.join(cwd, ".pylon");
    fs.mkdirSync(stageDir, { recursive: true });
    const entryPath = path.join(stageDir, "client-entry.tsx");
    const entrySource = generateEntrySource(routes);
    fs.writeFileSync(entryPath, entrySource, "utf8");

    const outdir = path.join(stageDir, "client-build");
    fs.mkdirSync(outdir, { recursive: true });

    const result = await Bun.build({
      entrypoints: [entryPath],
      outdir,
      target: "browser",
      format: "esm",
      minify: true,
      sourcemap: "none",
    });

    if (!result.success) {
      const msgs = (result.logs ?? [])
        .map((l) => `${l.level}: ${l.message}`)
        .join("\n");
      throw new Error(`Bun.build failed:\n${msgs || "(no log messages)"}`);
    }
    const jsOut = result.outputs.find(
      (o) =>
        o.path.endsWith(".js") || o.path.endsWith(".mjs") || o.kind === "entry-point",
    );
    if (!jsOut) {
      throw new Error("Bun.build produced no JS output");
    }
    // Bun.build writes outputs to disk when `outdir` is set, so
    // `jsOut.path` is already an absolute path we can hand to the
    // host. Confirm it exists before reporting success.
    if (!fs.existsSync(jsOut.path)) {
      throw new Error(`Bun.build reported ${jsOut.path} but it's not on disk`);
    }

    send({
      type: "bundle_client_result",
      call_id: msg.call_id,
      path: jsOut.path,
    });
  } catch (err: any) {
    send({
      type: "bundle_client_result",
      call_id: msg.call_id,
      path: "",
      error: err?.message || String(err),
    });
  }
}
