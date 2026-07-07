// Route discovery (discoverAppRoutes) — the app/ file-convention walk. Focus
// here is the dynamic OG image convention (app/**/opengraph-image.tsx →
// kind:"og-image"), which must produce a matchable route at
// `<segment>/opengraph-image` (root → `/opengraph-image`) alongside the page.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { discoverAppRoutes } from "./index";

describe("discoverAppRoutes — opengraph-image convention", () => {
  const tmpdirs: string[] = [];
  const prevCwd = process.cwd();
  afterEach(() => {
    process.chdir(prevCwd);
    for (const d of tmpdirs.splice(0)) {
      try {
        fs.rmSync(d, { recursive: true, force: true });
      } catch {
        /* best effort */
      }
    }
  });

  // Materialize an app/ tree (relative file paths → contents) and chdir in.
  function app(files: Record<string, string>): void {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-routes-"));
    tmpdirs.push(root);
    for (const [rel, src] of Object.entries(files)) {
      const abs = path.join(root, "app", rel);
      fs.mkdirSync(path.dirname(abs), { recursive: true });
      fs.writeFileSync(abs, src);
    }
    process.chdir(root);
  }

  const PAGE = "export default function P(){ return null; }";
  const OG = "export default function OG(){ return null; }";

  test("root opengraph-image → /opengraph-image kind:og-image", async () => {
    app({ "page.tsx": PAGE, "opengraph-image.tsx": OG });
    const routes = await discoverAppRoutes();
    const og = routes.find((r) => r.path === "/opengraph-image");
    expect(og).toBeDefined();
    expect(og!.kind).toBe("og-image");
    expect(og!.mode).toBe("ssr");
    // The component points at the module (not "/page").
    expect(og!.component).toMatch(/opengraph-image$/);
  });

  test("nested + dynamic-param segment → concrete pattern path", async () => {
    app({
      "page.tsx": PAGE,
      "blog/[slug]/page.tsx": PAGE,
      "blog/[slug]/opengraph-image.tsx": OG,
    });
    const routes = await discoverAppRoutes();
    const og = routes.find((r) => r.kind === "og-image");
    expect(og).toBeDefined();
    // Dynamic segment tokenizes to `:slug`, matching the page route pattern.
    expect(og!.path).toBe("/blog/:slug/opengraph-image");
  });

  test("multiple opengraph-images coexist (root default + per-section)", async () => {
    app({
      "page.tsx": PAGE,
      "opengraph-image.tsx": OG,
      "shop/page.tsx": PAGE,
      "shop/opengraph-image.tsx": OG,
    });
    const routes = await discoverAppRoutes();
    const ogPaths = routes
      .filter((r) => r.kind === "og-image")
      .map((r) => r.path)
      .sort();
    expect(ogPaths).toEqual(["/opengraph-image", "/shop/opengraph-image"]);
  });

  test("no opengraph-image → no og-image routes", async () => {
    app({ "page.tsx": PAGE });
    const routes = await discoverAppRoutes();
    expect(routes.some((r) => r.kind === "og-image")).toBe(false);
  });
});
