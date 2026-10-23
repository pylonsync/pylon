// Build-time brotli and gzip copies of the client build's text files.

import { afterEach, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { MIN_COMPRESS_BYTES, precompressBuild } from "./ssr-precompress";

let dir = "";
afterEach(() => {
  if (dir) fs.rmSync(dir, { recursive: true, force: true });
  dir = "";
});

const js = "export const rows = [" + Array.from({ length: 400 }, (_, i) => `{id:${i},name:"row ${i}"}`).join(",") + "];\n";

test("writes .br and .gz copies that decompress to the file", () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-precompress-"));
  fs.mkdirSync(path.join(dir, "chunks"));
  fs.writeFileSync(path.join(dir, "chunks", "chunk-abcd1234.js"), js);
  fs.writeFileSync(path.join(dir, "styles-abcd1234.css"), "a{color:red}".repeat(200));

  const result = precompressBuild(fs, path, zlib, dir);
  expect(result.files).toBe(2);
  expect(result.brotliBytes).toBeLessThan(result.bytes);

  const file = path.join(dir, "chunks", "chunk-abcd1234.js");
  expect(zlib.brotliDecompressSync(fs.readFileSync(file + ".br")).toString()).toBe(js);
  expect(zlib.gunzipSync(fs.readFileSync(file + ".gz")).toString()).toBe(js);
  expect(fs.statSync(file + ".br").size).toBeLessThan(js.length / 4);
});

test("skips small files, binary formats and copies that would not be smaller", () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "pylon-precompress-"));
  fs.writeFileSync(path.join(dir, "tiny-abcd1234.js"), "x".repeat(MIN_COMPRESS_BYTES - 1));
  fs.writeFileSync(path.join(dir, "font-abcd1234.woff2"), js);
  // Random bytes do not compress.
  const noise = Buffer.from(Array.from({ length: 4096 }, () => Math.floor(Math.random() * 256)));
  fs.writeFileSync(path.join(dir, "noise-abcd1234.js"), noise);

  const result = precompressBuild(fs, path, zlib, dir);
  expect(result.files).toBe(0);
  expect(fs.readdirSync(dir).sort()).toEqual([
    "font-abcd1234.woff2",
    "noise-abcd1234.js",
    "tiny-abcd1234.js",
  ]);
});
