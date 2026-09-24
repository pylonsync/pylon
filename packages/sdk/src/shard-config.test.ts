/**
 * `shard({...})` and `buildManifest({ shards })`: shard kinds reach the
 * manifest as written, and a value the runtime cannot use fails when app.ts
 * runs.
 */
import { expect, test } from "bun:test";
import { buildManifest, entity, field, shard } from "./index";

const base = {
  name: "t",
  version: "0",
  entities: [entity("Doc", { title: field.string() })],
  routes: [],
};

test("shard kinds are copied into the manifest", () => {
  const arena = shard({
    name: "arena",
    wasm: "shards/arena.wasm",
    crate: "shards/arena",
    codec: "msgpack",
    tickRate: 30,
    maxInstances: 8,
    input: { ratePerSec: 60 },
  });
  expect(buildManifest({ ...base, shards: [arena] }).shards).toEqual([arena]);
  expect("shards" in buildManifest(base)).toBe(false);
});

test("bad shard kinds throw", () => {
  expect(() => shard({ name: "1bad", wasm: "a.wasm" })).toThrow("name must start");
  expect(() => shard({ name: "a", wasm: "../a.wasm" })).toThrow("relative to the app root");
  expect(() => shard({ name: "a", wasm: "/abs/a.wasm" })).toThrow("relative to the app root");
  expect(() => shard({ name: "a", wasm: "a.js" })).toThrow(".wasm file");
  expect(() => shard({ name: "a", wasm: "a.wasm", codec: "bincode" as never })).toThrow("codec");
  expect(() => shard({ name: "a", wasm: "a.wasm", tickRate: 1.5 })).toThrow("tickRate");
  expect(() => shard({ name: "a", wasm: "a.wasm", memoryMb: 0 })).toThrow("memoryMb");
  const a = { name: "a", wasm: "a.wasm" };
  expect(() => buildManifest({ ...base, shards: [a, a] })).toThrow('two shard kinds are named "a"');
});
