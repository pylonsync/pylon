// SSR module lookups in a production server bundle.
//
// An artifact has no app/ source files, so every lookup that checks the disk
// in source mode (boundary walk, route-group dirs, layout chain, module
// import) must answer from the bundle's module registry instead. These run
// the lookups with a registry set and a cwd that holds no files, and expect
// the same answers the source-mode tests get from a real tree.

import { afterAll, afterEach, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { moduleKey, setServerBundle, type PylonServerBundle } from "./server-bundle";
import { findBoundaryIn, importModule, moduleExistsIn } from "./ssr-runtime";

const EMPTY_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-bundle-lookup-"));

/** Set a registry with one loader per key. Returns the module namespaces the
 *  loaders resolve to, and how many times each loader ran. */
function withModules(keys: string[]): { ns: Record<string, any>; loads: Record<string, number> } {
  const ns: Record<string, any> = {};
  const loads: Record<string, number> = {};
  const modules: PylonServerBundle["modules"] = {};
  for (const k of keys) {
    ns[k] = { default: () => k };
    loads[k] = 0;
    modules[k] = async () => {
      loads[k] += 1;
      return ns[k];
    };
  }
  const bundle: PylonServerBundle = {
    functions: {},
    workflows: {},
    modules,
    react: null,
    reactDomServer: null,
    clientDir: "client",
    ogAssets: { resvgWasm: "", interRegular: "", interSemiBold: "" },
  };
  setServerBundle(bundle);
  return { ns, loads };
}

afterEach(() => {
  delete (globalThis as any).__PYLON_SERVER_BUNDLE__;
});
afterAll(() => fs.rmSync(EMPTY_DIR, { recursive: true, force: true }));

test("moduleKey normalizes separators, a leading ./, and extensions", () => {
  expect(moduleKey(".\\app\\blog\\page.tsx")).toBe("app/blog/page");
  expect(moduleKey("app/page")).toBe("app/page");
});

test("moduleExistsIn answers from the registry, not the disk", () => {
  withModules(["app/layout", "app/blog/page"]);
  expect(moduleExistsIn(fs, path, EMPTY_DIR, "app", "layout")).toBe(true);
  expect(moduleExistsIn(fs, path, EMPTY_DIR, "app/blog", "page")).toBe(true);
  expect(moduleExistsIn(fs, path, EMPTY_DIR, "app/blog", "layout")).toBe(false);
});

test("the boundary walk finds the nearest boundary, through route groups", () => {
  withModules([
    "app/not-found",
    "app/(shop)/cart/not-found",
    "app/(shop)/cart/items/page",
    "app/(marketing)/error",
    "app/blog/page",
  ]);
  expect(findBoundaryIn(fs, path, EMPTY_DIR, "app/(shop)/cart/items/page", "not-found")).toBe(
    "app/(shop)/cart/not-found",
  );
  expect(findBoundaryIn(fs, path, EMPTY_DIR, "app/blog/page", "not-found")).toBe("app/not-found");
  // A group directly under app/ answers for "/" too.
  expect(findBoundaryIn(fs, path, EMPTY_DIR, "app/blog/page", "error")).toBe(
    "app/(marketing)/error",
  );
  expect(findBoundaryIn(fs, path, EMPTY_DIR, "app/blog/page", "loading")).toBeNull();
});

test("importModule loads the bundled module on demand and names a missing one", async () => {
  const { ns, loads } = withModules(["app/page", "app/layout"]);
  expect(await importModule(EMPTY_DIR, "app/page")).toBe(ns["app/page"]);
  // Lookups by key never evaluate a module; only an import does.
  expect(moduleExistsIn(fs, path, EMPTY_DIR, "app", "layout")).toBe(true);
  expect(loads).toEqual({ "app/page": 1, "app/layout": 0 });
  await expect(importModule(EMPTY_DIR, "app/missing")).rejects.toThrow("not in the server bundle");
});
