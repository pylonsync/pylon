// The route's modulepreload set is its whole static import graph, including
// chunk-to-chunk imports (a chunk imports its siblings as "./x.js").

import { afterEach, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { staticImportGraph } from "./ssr-client-bundler";

let dir = "";
afterEach(() => {
  if (dir) fs.rmSync(dir, { recursive: true, force: true });
});

test("follows chunk-to-chunk imports, not dynamic ones", () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-graph-"));
  fs.mkdirSync(path.join(dir, "chunks"));
  const write = (f: string, body: string) => fs.writeFileSync(path.join(dir, f), body);
  write("client-entry-home-aaaa1111.js", `import{a}from"./chunks/a-11111111.js";import"./chunks/b-22222222.js";a();`);
  write("chunks/a-11111111.js", `import{c as d}from"./c-33333333.js";export const a=()=>d();`);
  write("chunks/b-22222222.js", `import"./c-33333333.js";`);
  write("chunks/c-33333333.js", `import"./d-44444444.js";export const c=1;const later=()=>import("./lazy-55555555.js");`);
  write("chunks/d-44444444.js", `export const d=1;`);
  write("chunks/lazy-55555555.js", `export const lazy=1;`);
  const chunks = new Set(
    ["a-11111111", "b-22222222", "c-33333333", "d-44444444", "lazy-55555555"].map((c) => `chunks/${c}.js`),
  );
  const graph = staticImportGraph(fs, path, dir, path.join(dir, "client-entry-home-aaaa1111.js"), chunks);
  expect([...graph]).toEqual([
    "chunks/a-11111111.js",
    "chunks/b-22222222.js",
    "chunks/c-33333333.js",
    "chunks/d-44444444.js",
  ]);
});
