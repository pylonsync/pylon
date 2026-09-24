// Client browser compatibility (build.target / polyfill / css.target).
//
// The pure helpers are tested directly. `applyClientCompat` and
// `transformCss` run the real tools (SWC, browserslist, core-js, Lightning
// CSS), resolved from this package's devDependencies, against files in a
// temp dir.

import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  applyClientCompat,
  browsersBelowFloor,
  extractCoreJsImports,
  rehashName,
  resolveClientCompat,
  targetQueries,
  transformCss,
} from "./build-compat";

const PKG_DIR = path.resolve(import.meta.dir, "..");
let tmp: string | null = null;
afterEach(() => {
  if (tmp) fs.rmSync(tmp, { recursive: true, force: true });
  tmp = null;
});

describe("targetQueries", () => {
  test("an ECMAScript edition becomes browser versions", () => {
    expect(targetQueries("es2019", "build.target")).toContain("safari 12.1");
  });

  test("browserslist queries pass through", () => {
    expect(targetQueries(["safari >= 14", "chrome >= 90"], "t")).toEqual([
      "safari >= 14",
      "chrome >= 90",
    ]);
  });

  test("editions below es2018 are rejected: the runtime needs ES modules", () => {
    expect(() => targetQueries("es2015", "build.target")).toThrow("es2018");
  });

  test("empty values are rejected", () => {
    expect(() => targetQueries([], "build.target")).toThrow("must not be empty");
    expect(() => targetQueries([" "], "build.target")).toThrow("non-empty");
  });
});

describe("resolveClientCompat", () => {
  test("no target and no css target → no compat pass", () => {
    expect(resolveClientCompat(undefined)).toBeNull();
    expect(resolveClientCompat({ server: { external: ["x"] } })).toBeNull();
  });

  test("css.target defaults to target", () => {
    const c = resolveClientCompat({ target: "es2020" })!;
    expect(c.cssTargets).toEqual(c.jsTargets);
  });

  test("css.target alone lowers CSS only", () => {
    const c = resolveClientCompat({ css: { target: "safari >= 13" } })!;
    expect(c.jsTargets).toEqual([]);
    expect(c.cssTargets).toEqual(["safari >= 13"]);
  });

  test("polyfill without target is an error", () => {
    expect(() => resolveClientCompat({ polyfill: "usage" })).toThrow("needs build.target");
  });
});

test("extractCoreJsImports strips minified and spaced forms", () => {
  const found = new Set<string>();
  const code =
    'import"core-js/modules/es.array.at.js";import "core-js/modules/es.object.has-own.js";\nconst a=1;';
  expect(extractCoreJsImports(code, found).trim()).toBe("const a=1;");
  expect([...found].sort()).toEqual([
    "core-js/modules/es.array.at.js",
    "core-js/modules/es.object.has-own.js",
  ]);
});

test("rehashName changes the hash with the salt and keeps the stem", () => {
  const h = (s: string) => Bun.hash(s).toString(16).padStart(16, "0");
  const a = rehashName("client-entry-app__page-abc123.js", "es2019", h);
  const b = rehashName("client-entry-app__page-abc123.js", "es2020", h);
  expect(a).toMatch(/^client-entry-app__page-[0-9a-f]{10}\.js$/);
  expect(a).not.toBe(b);
  expect(rehashName("loro_wasm_bg.wasm", "x", h)).toBe("loro_wasm_bg.wasm");
});

test("browsersBelowFloor lists browsers that cannot load ES module entries", () => {
  expect(browsersBelowFloor(["chrome 90", "safari 10.1", "ie 11", "ios_saf 15.2-15.3"])).toEqual([
    "safari 10.1",
    "ie 11",
  ]);
});

describe("applyClientCompat (real SWC + core-js)", () => {
  function outdirWith(files: Record<string, string>): string {
    // Inside the package so the tools resolve from its node_modules.
    tmp = fs.mkdtempSync(path.join(PKG_DIR, ".compat-test-"));
    const out = path.join(tmp, "client-build");
    for (const [rel, code] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(out, rel)), { recursive: true });
      fs.writeFileSync(path.join(out, rel), code);
    }
    return out;
  }

  test("lowers syntax, bundles usage polyfills, and renames every output", async () => {
    const out = outdirWith({
      "client-entry-app__page-aaaa1111.js":
        'import{x as t}from"./chunks/shared-bbbb2222.js";export const v=t?.y??[1].at(-1);',
      "chunks/shared-bbbb2222.js": "export const x={y:Object.hasOwn({},'a')};",
    });
    const res = await applyClientCompat({
      fs,
      path,
      cwd: tmp!,
      outdir: out,
      jsFiles: ["client-entry-app__page-aaaa1111.js", "chunks/shared-bbbb2222.js"],
      compat: {
        jsTargets: ["chrome 70", "safari 12"],
        cssTargets: [],
        polyfill: "usage",
        sourcemap: false,
      },
    });

    const entryRel = res.renamed.get("client-entry-app__page-aaaa1111.js")!;
    const chunkRel = res.renamed.get("chunks/shared-bbbb2222.js")!;
    expect(entryRel).toMatch(/^client-entry-app__page-[0-9a-f]{10}\.js$/);
    expect(chunkRel).toMatch(/^chunks\/shared-[0-9a-f]{10}\.js$/);
    expect(fs.existsSync(path.join(out, "client-entry-app__page-aaaa1111.js"))).toBe(false);

    const entry = fs.readFileSync(path.join(out, entryRel), "utf8");
    expect(entry).not.toContain("?.");
    expect(entry).not.toContain("??");
    expect(entry).not.toContain("core-js");
    // The reference to the renamed chunk was rewritten.
    expect(entry).toContain(path.basename(chunkRel));
    expect(entry).not.toContain("shared-bbbb2222.js");

    expect(res.polyfills).toMatch(/^polyfills-[0-9a-f]{10}\.js$/);
    const poly = fs.readFileSync(path.join(out, res.polyfills!), "utf8");
    expect(poly.length).toBeGreaterThan(1000);
    expect(poly).not.toMatch(/^import/m);
    expect(res.unsupported).toEqual([]);
  });

  test("targets that need no polyfill produce no polyfill file", async () => {
    const out = outdirWith({ "client-entry-app__page-cccc3333.js": "export const v=1+1;" });
    const res = await applyClientCompat({
      fs,
      path,
      cwd: tmp!,
      outdir: out,
      jsFiles: ["client-entry-app__page-cccc3333.js"],
      compat: { jsTargets: ["chrome 120"], cssTargets: [], polyfill: "usage", sourcemap: false },
    });
    expect(res.polyfills).toBeUndefined();
  });
});

test("transformCss adds prefixes and lowers nesting for the targets", async () => {
  const css = ".a{user-select:none;&:hover{color:red}}";
  const out = await transformCss(PKG_DIR, css, "x.css", ["safari 13"]);
  expect(out).toContain("-webkit-user-select:none");
  expect(out).toContain(".a:hover");
});
