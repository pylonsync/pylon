/**
 * Shard Arena — a realtime shard whose game logic is Rust compiled to
 * WebAssembly.
 *
 * The `arena` shard kind runs `shards/arena.wasm`, built from the crate in
 * `shards/arena` (`pylon shards build`, or `pylon dev`, which rebuilds it
 * when the Rust changes). The stock pylon binary runs the module on the
 * shard's tick thread, on Pylon Cloud too.
 */
import { buildManifest, discoverAppRoutes, entity, field, shard } from "@pylonsync/sdk";

// A zone player's durable data. Zones load it on join and write x back;
// items come only from grantItem, once per key. No policies: clients cannot
// read or write these; zones and functions do.
const Character = entity("Character", {
  userId: field.string().unique(),
  x: field.int(),
  /** The number of the next grant, which makes its key. */
  nextGrant: field.int(),
});

const Item = entity(
  "Item",
  {
    characterId: field.string(),
    name: field.string(),
    grantKey: field.string(),
  },
  {
    indexes: [
      { name: "by_character", fields: ["characterId"], unique: false },
      { name: "by_grant", fields: ["grantKey"], unique: true },
    ],
  },
);

const GmAction = entity("GmAction", {
  zone: field.string(),
  userId: field.string(),
  action: field.string(),
});

const manifest = buildManifest({
  name: "shard-arena",
  version: "0.1.0",
  entities: [Character, Item, GmAction],
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
      // Room for `pylon bench shard --bots 300` (see README).
      maxSubscribers: 1000,
      // The lobby stays up with nobody in it.
      idleShutdownSecs: 0,
    }),
    // A large zone with interest management and entity replication: each
    // player gets only the players within its view radius, as deltas within
    // a byte budget. For `pylon bench shard --join joinFrontier`.
    shard({
      name: "frontier",
      wasm: "shards/frontier.wasm",
      crate: "shards/frontier",
      tickRate: 20,
      maxInstances: 4,
      maxSubscribers: 1000,
      idleShutdownSecs: 0,
    }),
    // Zones that players move between with their state (functions/moveZone.ts).
    shard({
      name: "zone",
      wasm: "shards/zone.wasm",
      crate: "shards/zone",
      tickRate: 20,
      maxInstances: 16,
      idleShutdownSecs: 0,
    }),
  ],
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
