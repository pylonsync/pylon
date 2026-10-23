// Regression tests for the Phase 1.5e SSR client bundler.
//
// These guard the shape we DEPEND on for shared chunks to actually
// save bytes:
//   1. multi-entry build with `splitting: true` produces a
//      `chunks/` subdirectory.
//   2. React lands in the shared chunk, NOT in any per-route entry
//      (otherwise we'd duplicate ~120KB per page).
//   3. the manifest names every discovered route, each pointing at
//      exactly one entry file with its preload-list of chunks.
//   4. adding a third page expands the manifest by one entry, and
//      the new entry stays small (proves splitting is doing work,
//      not just renaming the monolith).
//
// We exercise the bundler against a fixture app directory created
// fresh per test. This keeps the test self-contained and lets us
// verify behavior on real Bun.build outputs — no mocking.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

import {
  buildClientBundle,
  buildTailwind,
  assertNotServerOnly,
  componentPathFor,
  _doBuildInner,
  discoverLoadingModules,
  generateLoadingRegistry,
  type PylonBundleManifest,
} from "./ssr-client-bundler";
import {
  boundaryScope,
  nearestBoundaryComponent,
} from "./ssr-client-boundary";

// State that needs cleanup between tests.
let originalCwd: string | null = null;
let tempDir: string | null = null;

function makeFixture(pages: Record<string, string>, layouts: Record<string, string>): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-ssr-bundler-test-"));
  fs.mkdirSync(path.join(dir, "app"), { recursive: true });
  for (const [relPath, source] of Object.entries(pages)) {
    const full = path.join(dir, "app", relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, source, "utf8");
  }
  for (const [relPath, source] of Object.entries(layouts)) {
    const full = path.join(dir, "app", relPath);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, source, "utf8");
  }
  // The bundler resolves `react` and `react-dom/client` relative
  // to CWD. Point at `examples/ssr-hello/node_modules` — that
  // fixture installs them already and we don't want to maintain a
  // second copy.
  const wsRoot = path.resolve(import.meta.dir, "..", "..", "..");
  const reactSource = path.join(
    wsRoot,
    "examples",
    "ssr-hello",
    "node_modules",
  );
  if (!fs.existsSync(reactSource)) {
    throw new Error(
      `react fixture not present at ${reactSource}; run \`bun install\` inside examples/ssr-hello`,
    );
  }
  fs.symlinkSync(reactSource, path.join(dir, "node_modules"));
  return dir;
}

afterEach(() => {
  if (originalCwd) {
    process.chdir(originalCwd);
    originalCwd = null;
  }
  if (tempDir) {
    try {
      fs.rmSync(tempDir, { recursive: true, force: true });
    } catch {
      /* best-effort */
    }
    tempDir = null;
  }
});

const PAGE_BODY = (name: string) => `
import React, { useState } from "react";
export default function ${name}() {
  const [n] = useState(0);
  return <div data-page="${name.toLowerCase()}">Hello {n}</div>;
}
`;

const LAYOUT_BODY = `
import React from "react";
export default function Layout({ children }: { children: React.ReactNode }) {
  return <html><body>{children}</body></html>;
}
`;

describe("ssr-client-bundler (Phase 1.5e)", () => {
  test("multi-entry build emits per-route entries + shared chunks dir", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "hello/page.tsx": PAGE_BODY("Hello"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath, outdir } = await buildClientBundle();

    expect(fs.existsSync(manifestPath)).toBe(true);
    expect(fs.existsSync(outdir)).toBe(true);
    expect(fs.existsSync(path.join(outdir, "chunks"))).toBe(true);

    const entries = fs
      .readdirSync(outdir)
      .filter((n) => n.startsWith("client-entry-") && n.endsWith(".js"));
    expect(entries.length).toBe(2);

    const chunks = fs.readdirSync(path.join(outdir, "chunks"));
    expect(chunks.length).toBeGreaterThan(0);
  });

  test("react-dom appears in the shared chunk, not in any per-route entry", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "hello/page.tsx": PAGE_BODY("Hello"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { outdir } = await buildClientBundle();

    const entryFiles = fs
      .readdirSync(outdir)
      .filter((n) => n.startsWith("client-entry-") && n.endsWith(".js"))
      .map((n) => path.join(outdir, n));
    const chunkFiles = fs
      .readdirSync(path.join(outdir, "chunks"))
      .map((n) => path.join(outdir, "chunks", n));

    // The entry files should be TINY — just an import + hydrate
    // call. Anything > a few KB means react was inlined.
    for (const ef of entryFiles) {
      const src = fs.readFileSync(ef, "utf8");
      // No copy of react's reconciler ("Fiber") symbols should be
      // in the entry file.
      expect(src.length).toBeLessThan(5_000);
      expect(src).not.toMatch(/Fiber/);
    }

    // At least one chunk should contain the react-dom internals.
    const chunkHasReact = chunkFiles.some((cf) => {
      const src = fs.readFileSync(cf, "utf8");
      return /hydrateRoot|reactDom|react-dom/i.test(src);
    });
    expect(chunkHasReact).toBe(true);
  });

  test("client runtime wires the nav-fallback guard (uncaught render error → full page load)", async () => {
    // Regression: a page that renders React-19-hoisted <title>/<meta>/<link> in
    // its own tree (use the `metadata` export instead) makes a client-side nav
    // re-render throw in React's commit phase — the URL changes but the page
    // can't swap (white screen). hydrateRoot must carry an onUncaughtError that
    // falls back to a full page load of the in-flight destination so nav
    // degrades gracefully. The distinctive console string is preserved through
    // minification, so we assert the guard actually ships in the bundle.
    tempDir = makeFixture(
      { "page.tsx": PAGE_BODY("Home") },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { outdir } = await buildClientBundle();
    // The runtime (hydrateRoot + guard) lands in the shared chunk with multiple
    // entries, or inlined in the entry with one — read every emitted .js.
    const readJsRecursive = (dir: string): string[] =>
      fs.readdirSync(dir, { withFileTypes: true }).flatMap((d) => {
        const full = path.join(dir, d.name);
        if (d.isDirectory()) return readJsRecursive(full);
        return d.name.endsWith(".js") ? [fs.readFileSync(full, "utf8")] : [];
      });
    const bundled = readJsRecursive(outdir).join("\n");

    // onUncaughtError is a React hydrateRoot option (property name preserved).
    expect(bundled).toMatch(/onUncaughtError/);
    // The fallback path: a full page load of the pending destination.
    expect(bundled).toMatch(/falling back to a full page load/);
  });

  test("manifest names every route, each with a non-empty imports list", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "hello/page.tsx": PAGE_BODY("Hello"),
        "about/page.tsx": PAGE_BODY("About"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath } = await buildClientBundle();
    const manifest = JSON.parse(
      fs.readFileSync(manifestPath, "utf8"),
    ) as PylonBundleManifest;

    expect(manifest.public_prefix).toBe("/_pylon/build/");
    expect(Object.keys(manifest.routes).sort()).toEqual([
      "app/about/page",
      "app/hello/page",
      "app/page",
    ]);

    for (const [component, route] of Object.entries(manifest.routes)) {
      expect(route.file).toMatch(/^client-entry-.+\.js$/);
      expect(route.imports.length).toBeGreaterThan(0);
      for (const imp of route.imports) {
        // Should be outdir-relative.
        expect(imp).not.toMatch(/^\//);
        expect(imp).not.toMatch(/^\.\.\//);
      }
    }
  });

  test("Windows separators: the build succeeds and every manifest path uses /", async () => {
    // node:path on Windows returns "\"-separated relative paths. They used to
    // reach the generated entries as `import Page from "../app\page"`, where
    // `\p` is a string escape, so Bun.build failed with "Bundle failed" and
    // every page shipped without hydration (#15).
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "blog/page.tsx": PAGE_BODY("Blog"),
        "not-found.tsx": PAGE_BODY("Missing"),
        "loading.tsx": LOADING_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    const winPath = {
      ...path,
      relative: (from: string, to: string) =>
        path.relative(from, to).replace(/\//g, "\\"),
    };

    const { manifestPath } = await _doBuildInner(fs, winPath, tempDir, "app");
    const manifest = JSON.parse(
      fs.readFileSync(manifestPath, "utf8"),
    ) as PylonBundleManifest;

    expect(Object.keys(manifest.routes).sort()).toEqual([
      "app/blog/page",
      "app/not-found",
      "app/page",
    ]);
    expect(manifest.outdir).toBe(".pylon/client-build");
    for (const route of Object.values(manifest.routes)) {
      expect(route.file).not.toContain("\\");
      for (const imp of route.imports) expect(imp).not.toContain("\\");
    }
    const entry = fs.readFileSync(
      path.join(tempDir, ".pylon", "client-entry-app__blog__page.tsx"),
      "utf8",
    );
    expect(entry).toContain('import Page from "../app/blog/page";');
    expect(entry).toContain('import L0 from "../app/layout";');
  });

  test("componentPathFor returns a /-separated path for win32 input", () => {
    expect(
      componentPathFor(path.win32, "C:\\proj", "C:\\proj\\app\\(shop)\\cart\\page.tsx"),
    ).toBe("app/(shop)/cart/page");
  });

  test("adding a route grows the manifest by one and stays small per-entry", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "hello/page.tsx": PAGE_BODY("Hello"),
        "about/page.tsx": PAGE_BODY("About"),
        "settings/page.tsx": PAGE_BODY("Settings"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { outdir, manifestPath } = await buildClientBundle();
    const manifest = JSON.parse(
      fs.readFileSync(manifestPath, "utf8"),
    ) as PylonBundleManifest;

    expect(Object.keys(manifest.routes).length).toBe(4);

    // Per-entry file size cap. Each entry is just an import +
    // hydrate boilerplate; the dominant cost (React) is shared.
    // We allow generous slack for minifier variance but stay
    // well below the "would have been a monolith" baseline of
    // ~320KB.
    for (const route of Object.values(manifest.routes)) {
      const full = path.join(outdir, route.file);
      const size = fs.statSync(full).size;
      expect(size).toBeLessThan(5_000);
    }

    // All routes should reference the SAME shared chunk so the
    // browser caches it once and reuses across navigations.
    const sharedImports = Object.values(manifest.routes).map(
      (r) => r.imports.join("|"),
    );
    const unique = new Set(sharedImports);
    expect(unique.size).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Client error/not-found boundary. Regression coverage for the blank-page bug:
// a notFound() (or any error) thrown DURING a client render — the normal case
// when an async lookup 404s after hydration — used to propagate uncaught and
// blank the whole document because the client runtime wrapped pages in NO
// error boundary. nearestBoundaryComponent() is the resolution the runtime
// inlines; the build test guards the boundary wiring + not-found.tsx entry.
// ---------------------------------------------------------------------------

describe("imported assets", () => {
  // loro-crdt's browser build imports its .wasm; Bun turns that into a URL
  // string. It must be absolute under the build prefix and name a file the
  // build wrote, or the page fetches it relative to its own path and 404s.
  test("an imported .wasm gets an absolute URL that exists in the build", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "hello/page.tsx": PAGE_BODY("Hello"),
        "docs/[id]/page.tsx": `
import React from "react";
import wasmUrl from "./engine.wasm";
export default function Doc() {
  return <div data-wasm={wasmUrl}>doc</div>;
}
`,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    fs.writeFileSync(path.join(tempDir, "app", "docs", "[id]", "engine.wasm"), new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { outdir, manifestPath } = await buildClientBundle();

    const built = [
      ...fs.readdirSync(outdir).filter((n) => n.endsWith(".js")).map((n) => path.join(outdir, n)),
      ...fs.readdirSync(path.join(outdir, "chunks")).map((n) => path.join(outdir, "chunks", n)),
    ]
      .map((f) => fs.readFileSync(f, "utf8"))
      .join("\n");
    const urls = built.match(/["'](\/_pylon\/build\/assets\/engine-[A-Za-z0-9]+\.wasm)["']/);
    expect(urls).not.toBeNull();
    const rel = urls![1].slice("/_pylon/build/".length);
    expect(fs.existsSync(path.join(outdir, rel))).toBe(true);
    // Every import between built files resolves to a file the build wrote,
    // the way the browser resolves it (relative to the importing file).
    const jsFiles = [
      ...fs.readdirSync(outdir).filter((n) => n.endsWith(".js")).map((n) => path.join(outdir, n)),
      ...fs.readdirSync(path.join(outdir, "chunks")).filter((n) => n.endsWith(".js")).map((n) => path.join(outdir, "chunks", n)),
    ];
    for (const file of jsFiles) {
      const src = fs.readFileSync(file, "utf8");
      for (const m of src.matchAll(/(?:from\s*|import\s*\(?\s*)["']([^"']+\.js)["']/g)) {
        const spec = m[1];
        const target = spec.startsWith("/_pylon/build/")
          ? path.join(outdir, spec.slice("/_pylon/build/".length))
          : path.resolve(path.dirname(file), spec);
        expect(`${path.relative(outdir, file)} -> ${spec}: ${fs.existsSync(target)}`).toBe(`${path.relative(outdir, file)} -> ${spec}: true`);
      }
    }
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    expect(manifest.public_prefix).toBe("/_pylon/build/");
    for (const route of Object.values(manifest.routes)) {
      expect(route.imports.length).toBeGreaterThan(0);
      for (const imp of route.imports) expect(fs.existsSync(path.join(outdir, imp))).toBe(true);
    }
  });
});

describe("assets a stylesheet references", () => {
  // The asset plugin answers JavaScript imports with a JavaScript module; a
  // stylesheet's url(...) must stay with Bun, or the build fails with
  // "Cannot import a .js file into a CSS file".
  test("a page importing CSS that uses url() still builds", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": `
import React from "react";
import "./look.css";
export default function Home() {
  return <div className="hero">home</div>;
}
`,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    fs.writeFileSync(path.join(tempDir, "app", "look.css"), ".hero { background: url(./bg.png); }\n");
    fs.writeFileSync(path.join(tempDir, "app", "bg.png"), new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]));
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath } = await buildClientBundle();
    expect(fs.existsSync(manifestPath)).toBe(true);
  });
});

describe("nearestBoundaryComponent (client boundary resolution)", () => {
  const routes = new Set([
    "web/app/not-found",
    "web/app/dashboard/not-found",
    "web/app/dashboard/error",
    "web/app/dashboard/orgs/[slug]/page",
    "web/app/page",
  ]);

  test("returns the NEAREST ancestor boundary, not a farther one", () => {
    // [slug]/page has no own boundary → nearest not-found is dashboard's,
    // NOT the app-root one.
    expect(
      nearestBoundaryComponent(
        "web/app/dashboard/orgs/[slug]/page",
        "not-found",
        routes,
      ),
    ).toBe("web/app/dashboard/not-found");
  });

  test("falls back to a farther ancestor when no nearer one exists", () => {
    // A page outside /dashboard only has the app-root not-found.
    expect(
      nearestBoundaryComponent("web/app/page", "not-found", routes),
    ).toBe("web/app/not-found");
    // settings/page is under /dashboard but above no closer boundary → dashboard.
    expect(
      nearestBoundaryComponent(
        "web/app/dashboard/settings/page",
        "not-found",
        routes,
      ),
    ).toBe("web/app/dashboard/not-found");
  });

  test("resolves error.tsx independently of not-found.tsx", () => {
    expect(
      nearestBoundaryComponent(
        "web/app/dashboard/orgs/[slug]/page",
        "error",
        routes,
      ),
    ).toBe("web/app/dashboard/error");
    // No error.tsx outside /dashboard → null (runtime renders its default).
    expect(nearestBoundaryComponent("web/app/page", "error", routes)).toBeNull();
  });

  test("returns null when the app ships no boundary at all", () => {
    expect(
      nearestBoundaryComponent("web/app/page", "not-found", new Set(["web/app/page"])),
    ).toBeNull();
  });

  test("does not match a boundary in a sibling/unrelated subtree", () => {
    const r = new Set(["web/app/admin/not-found", "web/app/dashboard/page"]);
    // dashboard/page must NOT pick up admin's not-found.
    expect(
      nearestBoundaryComponent("web/app/dashboard/page", "not-found", r),
    ).toBeNull();
  });
});

const NOT_FOUND_BODY = `
import React from "react";
export default function NotFound() {
  return <div data-boundary="not-found">Not found</div>;
}
`;

describe("client boundary is wired into the build", () => {
  test("not-found.tsx gets its own manifest entry AND the runtime wraps pages in the error boundary", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "dashboard/page.tsx": PAGE_BODY("Dash"),
        "dashboard/not-found.tsx": NOT_FOUND_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath, outdir } = await buildClientBundle();
    const manifest = JSON.parse(
      fs.readFileSync(manifestPath, "utf8"),
    ) as PylonBundleManifest;

    // The not-found boundary must be a first-class route entry so the client
    // can resolve + dynamically load it on a client-thrown notFound().
    expect(manifest.routes["app/dashboard/not-found"]).toBeTruthy();

    // The shared runtime chunk must contain the boundary wiring — without it,
    // a client-thrown notFound() blanks the page (the bug this fixes).
    const sharedChunks = fs
      .readdirSync(path.join(outdir, "chunks"))
      .map((n) => fs.readFileSync(path.join(outdir, "chunks", n), "utf8"));
    const allShared = sharedChunks.join("\n");
    expect(allShared).toContain("PYLON_NOT_FOUND");
    expect(allShared).toContain("getDerivedStateFromError");
  });
});

describe("Tailwind compile is concurrency-safe", () => {
  // Regression: `pylon dev` warms the SSR bundle in one runner process while the
  // first requests drive their own rebuild in another. Both compiled Tailwind to
  // a SHARED temp file (`.styles.build.css`); whichever renamed it first won, and
  // the other's `renameSync` ENOENT'd → the compile threw → the route shipped
  // with no `css` → the page rendered UNSTYLED. Reproduced ~25% of cold boots
  // under concurrent load. The fix gives each compile a process-unique temp path
  // and publishes (rename) before pruning others. Drive several compiles at once
  // against one outdir and prove they all succeed.
  test("concurrent compiles against one outdir all produce the stylesheet", async () => {
    const root = makeFixture(
      { "page.tsx": PAGE_BODY("Home") },
      { "layout.tsx": LAYOUT_BODY },
    );
    tempDir = root; // registers it for afterEach cleanup
    // Opt into Tailwind: a real globals.css the CLI can compile. `@tailwindcss/cli`
    // + `tailwindcss` resolve through the fixture's node_modules symlink (it points
    // at examples/ssr-hello/node_modules, which declares both).
    fs.writeFileSync(
      path.join(root, "app", "globals.css"),
      '@import "tailwindcss";\n@source "../app/**/*.{tsx,ts,jsx,js}";\n',
      "utf8",
    );
    originalCwd = process.cwd();
    process.chdir(root);

    const outdir = path.join(root, ".pylon", "client-build");
    fs.mkdirSync(outdir, { recursive: true });

    // Old code: at least one of these rejects with the rename ENOENT.
    const results = await Promise.all(
      Array.from({ length: 4 }, () =>
        buildTailwind(fs, path, root, outdir, "app"),
      ),
    );

    for (const name of results) {
      expect(name).toMatch(/^styles-[a-z0-9]+\.css$/);
      expect(fs.existsSync(path.join(outdir, name as string))).toBe(true);
    }
    // Deterministic output → identical hash → one published asset, no leftover
    // temp files stranded in the outdir.
    expect(new Set(results).size).toBe(1);
    const stranded = (fs.readdirSync(outdir) as string[]).filter((f) =>
      f.startsWith(".styles.build."),
    );
    expect(stranded).toEqual([]);
  }, 20_000);
});

describe("server-only guard (secrets can't leak into the client bundle)", () => {
  test("assertNotServerOnly throws for server-only specifiers, passes others", () => {
    expect(() =>
      assertNotServerOnly("@pylonsync/functions/server-only", "app/page.tsx"),
    ).toThrow(/server-only/i);
    expect(() => assertNotServerOnly("server-only", "app/x/layout.tsx")).toThrow(
      /server-only/i,
    );
    // The importer is named so the author can find the offending module.
    expect(() => assertNotServerOnly("server-only", "app/secrets.ts")).toThrow(
      /app\/secrets\.ts/,
    );
    // Ordinary imports pass untouched.
    expect(() => assertNotServerOnly("react", "app/page.tsx")).not.toThrow();
    expect(() => assertNotServerOnly("@/lib/utils", "app/page.tsx")).not.toThrow();
  });

  test("a real client build importing a server-only module FAILS via the guard", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-server-only-"));
    const entry = path.join(dir, "entry.ts");
    fs.writeFileSync(entry, `import "server-only";\nexport const x = 1;\n`);
    // Same plugin the bundler installs. `guardFired` proves the guard's
    // onResolve ran for the marker (Bun runs plugin resolution BEFORE default
    // resolution) — distinguishing a real guard rejection from an unrelated
    // "module not found". Bun surfaces the plugin throw as a failed build.
    let guardFired = false;
    let failed = false;
    try {
      const result = await Bun.build({
        entrypoints: [entry],
        outdir: path.join(dir, "out"),
        target: "browser",
        plugins: [
          {
            name: "pylon-server-only",
            setup(build: any) {
              build.onResolve(
                { filter: /^(@pylonsync\/functions\/server-only|server-only)$/ },
                (args: any) => {
                  guardFired = true;
                  assertNotServerOnly(args.path, args.importer);
                },
              );
            },
          },
        ],
      } as any);
      failed = !result.success;
    } catch {
      failed = true;
    }
    expect(guardFired).toBe(true);
    expect(failed).toBe(true);
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

// ---------------------------------------------------------------------------
// loading.tsx during client-side navigation.
//
// Regression: a route-level loading.tsx was honored ONLY by the server's
// streaming render. The client bundler emitted entries for not-found and error
// but not loading, so no loading module existed on the client at all and a
// pending navigation left the previous page fully painted until the
// destination was ready. Polling the DOM every 15ms across a sidebar click
// never saw the skeleton; the same route hard-loaded streamed it fine.
// ---------------------------------------------------------------------------

const LOADING_BODY = `
import React from "react";
export default function Loading() {
  return <div aria-busy="true" data-skeleton="pylon-pending">Loading…</div>;
}
`;

describe("discoverLoadingModules", () => {
  test("finds loading modules at every depth, sorted", () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "loading.tsx": LOADING_BODY,
        "dashboard/page.tsx": PAGE_BODY("Dash"),
        "dashboard/events/[id]/page.tsx": PAGE_BODY("Event"),
        "dashboard/events/[id]/loading.tsx": LOADING_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    expect(discoverLoadingModules(fs, path, tempDir, "app")).toEqual([
      "app/dashboard/events/[id]/loading",
      "app/loading",
    ]);
  });

  test("finds one inside a route group", () => {
    // Chrome commonly lives in a (group) layout; its loading sibling has to be
    // discoverable by the same path walk the server uses.
    tempDir = makeFixture(
      {
        "(dash)/settings/page.tsx": PAGE_BODY("Settings"),
        "(dash)/loading.tsx": LOADING_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    expect(discoverLoadingModules(fs, path, tempDir, "app")).toEqual([
      "app/(dash)/loading",
    ]);
  });

  test("an app with no loading.tsx yields an empty list, not an error", () => {
    tempDir = makeFixture(
      { "page.tsx": PAGE_BODY("Home") },
      { "layout.tsx": LAYOUT_BODY },
    );
    expect(discoverLoadingModules(fs, path, tempDir, "app")).toEqual([]);
  });
});

describe("generateLoadingRegistry", () => {
  test("keys each module by component path so the nearest-ancestor walk works", () => {
    const src = generateLoadingRegistry([
      "app/loading",
      "app/dashboard/loading",
    ]);
    expect(src).toContain(`import M0 from "../app/loading";`);
    expect(src).toContain(`import M1 from "../app/dashboard/loading";`);
    expect(src).toContain(`"app/loading": M0,`);
    expect(src).toContain(`"app/dashboard/loading": M1,`);
  });

  test("emits a valid empty registry when the app has none", () => {
    // The runtime imports LOADING_MODULES unconditionally — an app without a
    // loading.tsx must still produce a module that parses and exports it.
    const src = generateLoadingRegistry([]);
    expect(src).toContain("export const LOADING_MODULES = {");
    expect(src).not.toContain("import M0");
  });
});

describe("nearestBoundaryComponent resolves loading.tsx", () => {
  const loadingKeys = new Set([
    "web/app/loading",
    "web/app/dashboard/events/[id]/loading",
  ]);

  test("a sibling route picks up the nearest ancestor's skeleton", () => {
    // Every tab under events/[id] shares that one loading.tsx.
    expect(
      nearestBoundaryComponent(
        "web/app/dashboard/events/[id]/speakers/page",
        "loading",
        loadingKeys,
      ),
    ).toBe("web/app/dashboard/events/[id]/loading");
  });

  test("falls back to the root skeleton outside that subtree", () => {
    expect(
      nearestBoundaryComponent("web/app/settings/page", "loading", loadingKeys),
    ).toBe("web/app/loading");
  });

  test("returns null when the app ships none", () => {
    expect(
      nearestBoundaryComponent("web/app/page", "loading", new Set()),
    ).toBeNull();
  });
});

describe("loading.tsx is wired into the client build", () => {
  test("ships in the SHARED chunk — a skeleton can't wait on its own fetch", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "dashboard/page.tsx": PAGE_BODY("Dash"),
        "dashboard/loading.tsx": LOADING_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath, outdir } = await buildClientBundle();
    const manifest = JSON.parse(
      fs.readFileSync(manifestPath, "utf8"),
    ) as PylonBundleManifest;

    // Not a route entry: it has no URL of its own, and a loading state that
    // costs a chunk fetch sits behind the very delay it exists to cover.
    expect(manifest.routes["app/dashboard/loading"]).toBeUndefined();

    const sharedChunks = fs
      .readdirSync(path.join(outdir, "chunks"))
      .map((n) => fs.readFileSync(path.join(outdir, "chunks", n), "utf8"))
      .join("\n");
    // The skeleton's own markup, and the registry key the runtime walks to.
    expect(sharedChunks).toContain("pylon-pending");
    expect(sharedChunks).toContain("app/dashboard/loading");
  });

  test("first hydration wraps the page in the same Suspense boundary the server renders", async () => {
    tempDir = makeFixture(
      {
        "dashboard/page.tsx": PAGE_BODY("Dash"),
        "dashboard/loading.tsx": LOADING_BODY,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);
    await buildClientBundle();
    const runtime = fs.readFileSync(
      path.join(tempDir, ".pylon", "client-runtime.ts"),
      "utf8",
    );
    // The hydrating tree is built with the page's component path, which
    // resolves its loading module and wraps the page in <Suspense>
    // (see ssr-hydration.test.ts for why a missing boundary breaks hydration).
    expect(runtime).toContain("buildTree(Page, Layouts, currentPageProps, data.component)");
    expect(runtime).toContain("createElement(Suspense, { fallback: createElement(Loading, props) }, tree)");
    // The metadata slot the server renders beside every page, empty in the
    // browser, from the same module the server uses (ssr-page-tree.ts).
    expect(runtime).toContain('import { withMetaSlot } from "./page-tree";');
    expect(runtime).toContain("tree = withMetaSlot(createElement, Fragment, null, tree);");
    expect(fs.readFileSync(path.join(tempDir, ".pylon", "page-tree.ts"), "utf8")).toBe(
      fs.readFileSync(path.join(import.meta.dir, "ssr-page-tree.ts"), "utf8"),
    );
  });

  test("an app with no loading.tsx still builds and navigates", async () => {
    // The runtime's import of ./loading-registry is unconditional, so the
    // staged module must exist even with nothing to put in it.
    tempDir = makeFixture(
      { "page.tsx": PAGE_BODY("Home"), "about/page.tsx": PAGE_BODY("About") },
      { "layout.tsx": LAYOUT_BODY },
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath } = await buildClientBundle();
    expect(fs.existsSync(manifestPath)).toBe(true);
    expect(
      fs.existsSync(path.join(tempDir, ".pylon", "loading-registry.ts")),
    ).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Route groups and boundary resolution.
//
// Regression: a `not-found.tsx` inside a route group served unmatched URLs
// (the host matches those on the manifest's URL path, where a group adds no
// segment, so it sits at "/") but NOT a notFound() thrown by a page (which
// walked real directories and never crossed out of the page's own subtree).
// So a group boundary silently covered one class of 404 and not the other,
// while ROUTE_PATH_DUPLICATE blocked keeping a root copy alongside it.
// ---------------------------------------------------------------------------

describe("boundaryScope", () => {
  test("drops group segments, keeps real ones", () => {
    expect(boundaryScope("web/app/(marketing)/pricing/page")).toBe("web/app/pricing");
    expect(boundaryScope("web/app/(marketing)/not-found")).toBe("web/app");
    expect(boundaryScope("app/not-found")).toBe("app");
  });

  test("handles nested groups", () => {
    expect(boundaryScope("app/(a)/(b)/not-found")).toBe("app");
  });
});

describe("nearestBoundaryComponent across route groups", () => {
  test("a group's boundary covers a page outside the group", () => {
    // Both are at URL "/", so the group copy IS the root boundary.
    expect(
      nearestBoundaryComponent(
        "app/gone/page",
        "not-found",
        new Set(["app/(marketing)/not-found", "app/gone/page"]),
      ),
    ).toBe("app/(marketing)/not-found");
  });

  test("a nested group's boundary resolves the same way", () => {
    expect(
      nearestBoundaryComponent(
        "app/gone/page",
        "not-found",
        new Set(["app/(a)/(b)/not-found"]),
      ),
    ).toBe("app/(a)/(b)/not-found");
  });

  test("a nearer real boundary still beats a group one higher up", () => {
    expect(
      nearestBoundaryComponent(
        "app/dashboard/settings/page",
        "not-found",
        new Set(["app/(marketing)/not-found", "app/dashboard/not-found"]),
      ),
    ).toBe("app/dashboard/not-found");
  });

  test("a group boundary does not leak into a deeper URL scope", () => {
    // app/(marketing)/pricing/not-found is at "/pricing" — it must not answer
    // for a page at "/dashboard".
    expect(
      nearestBoundaryComponent(
        "app/dashboard/page",
        "not-found",
        new Set(["app/(marketing)/pricing/not-found"]),
      ),
    ).toBeNull();
  });

  test("a boundary inside a group still covers that group's own pages", () => {
    expect(
      nearestBoundaryComponent(
        "app/(marketing)/pricing/page",
        "not-found",
        new Set(["app/(marketing)/not-found"]),
      ),
    ).toBe("app/(marketing)/not-found");
  });
});

describe("client router popstate", () => {
  test("a hash-only popstate does not re-render the route", () => {
    // Clicking a same-page anchor fires popstate. Treating it as a navigation
    // re-rendered the page and scrolled back to the top.
    const src = fs.readFileSync(path.join(import.meta.dir, "ssr-client-bundler.ts"), "utf8");
    const popstate = src.slice(src.indexOf('window.addEventListener("popstate"'));
    expect(popstate).toContain("if (next === renderedPath) return;");
    expect(src.match(/renderedPath = location\.pathname \+ location\.search;/g)?.length).toBeGreaterThanOrEqual(3);
  });
});

describe("offline service worker (buildManifest({ offline }))", () => {
  test("an offline app's build writes pylon-sw.js with each offline route's files", async () => {
    tempDir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "leads/page.tsx": PAGE_BODY("Leads"),
        "leads/[id]/page.tsx": PAGE_BODY("Lead"),
        "settings/page.tsx": PAGE_BODY("Settings"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    fs.writeFileSync(
      path.join(tempDir, "pylon.manifest.json"),
      JSON.stringify({
        offline: {
          name: "Field Sales",
          icons: [{ src: "/icons/192.png", sizes: "192x192" }],
          startUrl: "/leads",
          display: "standalone",
          routes: ["/leads", "/leads/[id]"],
        },
      }),
    );
    originalCwd = process.cwd();
    process.chdir(tempDir);

    const { manifestPath, outdir } = await buildClientBundle();
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    expect(manifest.offline?.routes).toEqual(["/leads", "/leads/[id]"]);
    expect(manifest.offline?.version).toMatch(/^[0-9a-f]{16}$/);

    const sw = fs.readFileSync(path.join(outdir, "pylon-sw.js"), "utf8");
    // Valid script (compiled, not run).
    const { Script } = await import("node:vm");
    expect(() => new Script(sw)).not.toThrow();
    const config = JSON.parse(sw.slice(sw.indexOf(")(self, ") + 8, sw.lastIndexOf(", function")));
    expect(config.version).toBe(manifest.offline!.version);
    expect(config.shells).toEqual(["/leads"]);
    const prefix = manifest.public_prefix;
    for (const component of ["app/leads/page", "app/leads/[id]/page"]) {
      expect(config.precache).toContain(prefix + manifest.routes[component].file);
    }
    // Not an offline route: its entry is not precached.
    expect(config.precache).not.toContain(prefix + manifest.routes["app/settings/page"].file);
    expect(config.precache).toContain("/icons/192.png");

    // `/_pylon/version` serves this file.
    const versionFile = JSON.parse(fs.readFileSync(path.join(outdir, "pylon-version.json"), "utf8"));
    expect(versionFile.version).toBe(manifest.offline!.version);
    expect(versionFile.builtAt).toBe(manifest.offline!.builtAt);
    expect(Number.isNaN(Date.parse(versionFile.builtAt))).toBe(false);
    expect(versionFile.install).toMatch(/^[0-9a-f]{16}$/);

    // The same build gives the same version: no reinstall per boot.
    const again = await buildClientBundle();
    const second = JSON.parse(fs.readFileSync(again.manifestPath, "utf8")) as PylonBundleManifest;
    expect(second.offline?.version).toBe(manifest.offline!.version);
  });

  test("a change to the app's name or icon changes the version and the install identity", async () => {
    const dir = makeFixture(
      { "page.tsx": PAGE_BODY("Home"), "leads/page.tsx": PAGE_BODY("Leads") },
      { "layout.tsx": LAYOUT_BODY },
    );
    tempDir = dir;
    const offline = {
      name: "Field Sales",
      icons: [{ src: "/icons/192.png", sizes: "192x192" }],
      startUrl: "/leads",
      display: "standalone",
      routes: ["/leads"],
    };
    const writeManifest = (o: object) =>
      fs.writeFileSync(path.join(dir, "pylon.manifest.json"), JSON.stringify({ offline: o }));
    fs.mkdirSync(path.join(dir, "public", "icons"), { recursive: true });
    fs.writeFileSync(path.join(dir, "public", "icons", "192.png"), "icon-a");
    writeManifest(offline);
    originalCwd = process.cwd();
    process.chdir(dir);

    const read = async () => {
      const { outdir } = await buildClientBundle();
      return JSON.parse(fs.readFileSync(path.join(outdir, "pylon-version.json"), "utf8"));
    };
    const base = await read();

    // New icon bytes at the same URL.
    fs.writeFileSync(path.join(dir, "public", "icons", "192.png"), "icon-b");
    const icon = await read();
    expect(icon.install).not.toBe(base.install);
    expect(icon.version).not.toBe(base.version);

    // A new name.
    writeManifest({ ...offline, name: "Veritas" });
    const named = await read();
    expect(named.install).not.toBe(icon.install);

    // The app's own manifest file wins over the config.
    fs.writeFileSync(
      path.join(dir, "public", "manifest.webmanifest"),
      JSON.stringify({ name: "Own", display: "standalone", icons: [] }),
    );
    const own = await read();
    expect(own.install).not.toBe(named.install);
    // A field iOS does not read at install (theme color) changes nothing.
    fs.writeFileSync(
      path.join(dir, "public", "manifest.webmanifest"),
      JSON.stringify({ name: "Own", display: "standalone", icons: [], theme_color: "#000" }),
    );
    expect((await read()).install).toBe(own.install);
  });

  test("an app without offline gets no worker", async () => {
    tempDir = makeFixture({ "page.tsx": PAGE_BODY("Home") }, { "layout.tsx": LAYOUT_BODY });
    originalCwd = process.cwd();
    process.chdir(tempDir);
    const { manifestPath, outdir } = await buildClientBundle();
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    expect(manifest.offline).toBeUndefined();
    expect(fs.existsSync(path.join(outdir, "pylon-sw.js"))).toBe(false);
  });
});

describe("instant navigation (export const navigation = \"client\")", () => {
  test("the manifest marks the page, and its entry passes the static metadata", async () => {
    const dir = makeFixture(
      {
        "page.tsx": PAGE_BODY("Home"),
        "leads/page.tsx":
          `export const navigation = "client";\nexport const metadata = { title: "Leads" };\n` +
          PAGE_BODY("Leads"),
        "reports/page.tsx": PAGE_BODY("Reports"),
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    tempDir = dir;
    originalCwd = process.cwd();
    process.chdir(dir);

    const { manifestPath, outdir } = await buildClientBundle();
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    expect(manifest.routes["app/leads/page"].navigation).toBe("client");
    expect(manifest.routes["app/reports/page"].navigation).toBeUndefined();
    expect(manifest.routes["app/page"].navigation).toBeUndefined();

    // The built client code carries the title for the runtime; a page without
    // static metadata imports none.
    const code = (component: string) => {
      const r = manifest.routes[component];
      return [r.file, ...r.imports]
        .map((f) => fs.readFileSync(path.join(outdir, f), "utf8"))
        .join("\n");
    };
    expect(code("app/leads/page")).toMatch(/title:\s*"Leads"/);
  });
});

describe("precompressed build files", () => {
  test("a production build writes .br and .gz copies of its chunks; precompress: false does not", async () => {
    const dir = makeFixture({ "page.tsx": PAGE_BODY("Home") }, { "layout.tsx": LAYOUT_BODY });
    tempDir = dir;
    originalCwd = process.cwd();
    process.chdir(dir);

    const { manifestPath, outdir } = await _doBuildInner(fs, path, dir, "app", { precompress: true });
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    const route = manifest.routes["app/page"];
    // The shared chunk holds React: large, and compressed.
    const shared = route.imports
      .map((f) => path.join(outdir, f))
      .sort((a, b) => fs.statSync(b).size - fs.statSync(a).size)[0];
    expect(fs.existsSync(shared + ".br")).toBe(true);
    expect(fs.existsSync(shared + ".gz")).toBe(true);
    expect(fs.statSync(shared + ".br").size).toBeLessThan(fs.statSync(shared).size / 2);

    const again = await _doBuildInner(fs, path, dir, "app", { precompress: false });
    const files = fs.readdirSync(again.outdir, { recursive: true }) as string[];
    expect(files.filter((f) => f.endsWith(".br") || f.endsWith(".gz"))).toEqual([]);
  });
});

describe("namespace barrels (radix-ui)", () => {
  test("a route bundles only the barrel packages it imports", async () => {
    const dir = makeFixture(
      {
        "alpha/page.tsx": `import { Alpha } from "fake-ui";\nexport default function P() { return <div>{Alpha.name}</div>; }\n`,
        "beta/page.tsx": `import { Beta as B } from "fake-ui";\nexport default function P() { return <div>{B.name}</div>; }\n`,
      },
      { "layout.tsx": LAYOUT_BODY },
    );
    tempDir = dir;
    // A barrel and the two packages it re-exports, resolvable from app/.
    const nm = path.join(dir, "app", "node_modules");
    const pkg = (name: string, files: Record<string, string>) => {
      fs.mkdirSync(path.join(nm, name), { recursive: true });
      for (const [f, body] of Object.entries(files)) fs.writeFileSync(path.join(nm, name, f), body);
    };
    pkg("fake-ui", {
      "package.json": JSON.stringify({ name: "fake-ui", module: "index.mjs", main: "index.mjs", sideEffects: false }),
      "index.mjs": `import * as Alpha from "@fake/alpha";\nimport * as Beta from "@fake/beta";\nexport { Alpha, Beta };\n`,
    });
    for (const [name, marker] of [["alpha", "ALPHA_ONLY_CODE"], ["beta", "BETA_ONLY_CODE"]]) {
      pkg(`@fake/${name}`, {
        "package.json": JSON.stringify({ name: `@fake/${name}`, module: "index.mjs", main: "index.mjs", sideEffects: false }),
        "index.mjs": `export const name = ${JSON.stringify(marker + " " + "x".repeat(2000))};\n`,
      });
    }
    originalCwd = process.cwd();
    process.chdir(dir);

    const { manifestPath, outdir } = await _doBuildInner(fs, path, dir, "app", { precompress: false });
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as PylonBundleManifest;
    const code = (component: string) => {
      const r = manifest.routes[component];
      return [r.file, ...r.imports].map((f) => fs.readFileSync(path.join(outdir, f), "utf8")).join("\n");
    };
    expect(code("app/alpha/page")).toContain("ALPHA_ONLY_CODE");
    expect(code("app/alpha/page")).not.toContain("BETA_ONLY_CODE");
    expect(code("app/beta/page")).toContain("BETA_ONLY_CODE");
    expect(code("app/beta/page")).not.toContain("ALPHA_ONLY_CODE");
  });
});
