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
