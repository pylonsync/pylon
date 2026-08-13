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
