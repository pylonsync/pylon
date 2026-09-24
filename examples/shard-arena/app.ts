/**
 * Shard Arena — a realtime shard whose game logic is Rust compiled to
 * WebAssembly.
 *
 * The `arena` shard kind runs `shards/arena.wasm`, built from the crate in
 * `shards/arena` (`pylon shards build`, or `pylon dev`, which rebuilds it
 * when the Rust changes). The stock pylon binary runs the module on the
 * shard's tick thread, on Pylon Cloud too.
 */
import { buildManifest, discoverAppRoutes, shard } from "@pylonsync/sdk";

const manifest = buildManifest({
  name: "shard-arena",
  version: "0.1.0",
  entities: [],
  queries: [],
  actions: [],
  policies: [],
  routes: await discoverAppRoutes(),
  shards: [
    shard({
      name: "arena",
      wasm: "shards/arena.wasm",
      crate: "shards/arena",
      codec: "msgpack",
      tickRate: 20,
      maxInstances: 4,
      // The lobby stays up with nobody in it.
      idleShutdownSecs: 0,
    }),
  ],
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
