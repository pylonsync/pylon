/**
 * Pylon Forge — collaborative 3D scene editor.
 *
 * Figma-for-3D: users spawn primitives (box, sphere, cone, torus),
 * drag them around on a grid, change color, delete. Every change is
 * a mutation that broadcasts via live query; presence cursors show
 * every other collaborator's pointer in 3D space.
 *
 * The scaling story:
 *   - Two entities (Prim + Cursor) with totally different update
 *     cadences — primitive edits are low-frequency, cursors are
 *     high-frequency
 *   - Single live query per entity serves the whole room
 *   - No custom realtime protocol — pure Pylon mutations + subs
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// A primitive in the scene.
const Prim = entity(
  "Prim",
  {
    roomId: field.string(),
    kind: field.string(),           // "box" | "sphere" | "cone" | "torus"
    x: field.float(),
    y: field.float(),
    z: field.float(),
    sx: field.float(),
    sy: field.float(),
    sz: field.float(),
    color: field.string(),
    createdBy: field.string(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_room", fields: ["roomId"], unique: false },
    ],
  },
);

// Per-user cursor presence — written at ~20 Hz while the pointer
// is over the scene.
const Cursor = entity(
  "Cursor",
  {
    roomId: field.string(),
    userId: field.string(),
    name: field.string(),
    color: field.string(),
    x: field.float(),
    y: field.float(),
    z: field.float(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_room_user", fields: ["roomId", "userId"], unique: true },
      { name: "by_room", fields: ["roomId"], unique: false },
    ],
  },
);

// Per-room terrain. One row per room holds the full heightmap + 4-layer
// splatmap as JSON strings. A 64x64 grid (the default) encodes to ~35 KB
// and re-serializes on each brush stroke. Clients throttle edits to 10 Hz
// so sync fan-out stays comfortable for a room of ~20 editors.
//
// Why JSON strings and not proper array fields: Pylon field types are
// scalars; nested arrays would require a sidecar entity or chunking. JSON
// lets us ship a working demo today; production MMO tooling would chunk
// terrain into 8x8 tiles keyed by (roomId, tileX, tileZ) so brush edits
// only touch the tiles they overlap.
const Terrain = entity(
  "Terrain",
  {
    roomId: field.string().unique(),
    size: field.int(),              // grid edge length in cells
    heights: field.string(),        // JSON number[][]
    layers: field.string(),         // JSON number[][][] — splatmap, 4 weights per cell
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_room", fields: ["roomId"], unique: true },
    ],
  },
);

const primPolicy = policy({
  name: "prim_room",
  entity: "Prim",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

const cursorPolicy = policy({
  name: "cursor_ownership",
  entity: "Cursor",
  allowRead: "true",
  allowInsert: "auth.userId == data.userId",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const terrainPolicy = policy({
  name: "terrain_room",
  entity: "Terrain",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
  allowDelete: "auth.userId != null",
});

const manifest = buildManifest({
  name: "forge",
  version: "0.1.0",
  entities: [Prim, Cursor, Terrain],
  queries: [],
  actions: [],
  policies: [primPolicy, cursorPolicy, terrainPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
