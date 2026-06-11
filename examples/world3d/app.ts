/**
 * Pylon World3D — multiplayer procedural-island FPS.
 *
 * A fully procedural tropical island (terrain, water, sky, clouds,
 * palms, grass, rocks) with destructible block buildings, rendered
 * in three.js and served by Pylon's native SSR — one binary, one port.
 *
 * Multiplayer rides on two entities:
 *   - Avatar       — one row per player, pose updated at ~10 Hz
 *   - Destruction  — one row per destroyed building block. The world
 *                    is deterministic from a fixed seed, so syncing
 *                    just the destroyed block keys reproduces the
 *                    exact same ruins on every client.
 */
import { buildManifest, discoverAppRoutes, entity, field, policy } from "@pylonsync/sdk";

const Avatar = entity(
  "Avatar",
  {
    userId: field.string(),
    name: field.string(),
    color: field.string(),
    x: field.float(),
    y: field.float(),
    z: field.float(),
    heading: field.float(), // radians, y-axis rotation
    pitch: field.float(),   // radians, look up/down (drives remote aim pose)
    // Optional so pre-combat rows don't break — clients treat null as
    // full health. Written ONLY by server functions (damage/respawn).
    health: field.float().optional(),
    lastSeenAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_user", fields: ["userId"], unique: true }],
  },
);

const Destruction = entity(
  "Destruction",
  {
    key: field.string(), // "<building>:<gx>:<gy>:<gz>" block coordinate key
    userId: field.string(),
    createdAt: field.datetime(),
  },
  {
    indexes: [{ name: "by_key", fields: ["key"], unique: true }],
  },
);

// World state is shared by design — every player sees every avatar and
// every destroyed block. Reads still require a session (guests get a
// userId from /api/auth/guest), so anonymous non-players can't scrape.
const avatarPolicy = policy({
  name: "avatar_ownership",
  entity: "Avatar",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const destructionPolicy = policy({
  name: "destruction_append_only",
  entity: "Destruction",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  // Rows are immutable facts; resetIsland (a function) deletes them.
  allowUpdate: "false",
  allowDelete: "auth.userId != null",
});

const manifest = buildManifest({
  name: "world3d",
  version: "0.2.0",
  entities: [Avatar, Destruction],
  queries: [],
  actions: [],
  policies: [avatarPolicy, destructionPolicy],
  routes: await discoverAppRoutes(),
});

// Emit canonical manifest JSON to stdout for pylon codegen.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
