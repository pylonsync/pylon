/**
 * Pylon Arena — mass-multiplayer dot world.
 *
 * Every connected client is a dot on a shared 2D plane. Click to set
 * a target; the dot glides there. All other clients see it move in
 * realtime. Stress-test by opening tabs or using the built-in bot
 * spawner — the whole point is to watch latency stay flat as N grows.
 *
 * The scaling story this demo tells:
 *   - One entity (`Dot`) with a live query fanned out to every client
 *   - Mutations land in <5ms p99 on a laptop
 *   - Hot path is single-digit KB per second per client
 *   - No sidecar, no Redis, no separate realtime layer
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// Each dot tracks its current position + target. Clients interpolate
// between updates, so we only need to write when the target changes.
const Dot = entity(
  "Dot",
  {
    userId: field.string(),
    x: field.float(),
    y: field.float(),
    tx: field.float(),
    ty: field.float(),
    color: field.string(),
    label: field.string().optional(),
    speed: field.float(),          // units per second
    isBot: field.bool(),
    lastSeenAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: true },
      { name: "by_bot", fields: ["isBot"], unique: false },
    ],
  },
);

// Per-instance counters bumped by functions so the stats panel can
// show live throughput without having to count rows each frame.
const ArenaStats = entity(
  "ArenaStats",
  {
    key: field.string().unique(),
    mutations: field.float(),
    broadcasts: field.float(),
    updatedAt: field.datetime(),
  },
);

// Open read (public demo). Writes require auth; you can only move
// your own dot unless you're spawning bots (writes tagged isBot: true
// are allowed since they're a demo feature).
const dotPolicy = policy({
  name: "dot_ownership",
  entity: "Dot",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId || data.isBot == true",
  allowDelete: "auth.userId == data.userId",
});

const statsPolicy = policy({
  name: "stats_public",
  entity: "ArenaStats",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
});

const manifest = buildManifest({
  name: "arena",
  version: "0.1.0",
  entities: [Dot, ArenaStats],
  queries: [],
  actions: [],
  policies: [dotPolicy, statsPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
