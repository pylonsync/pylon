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
  type PylonBundleManifest,
} from "./ssr-client-bundler";
import { nearestBoundaryComponent } from "./ssr-client-boundary";

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
