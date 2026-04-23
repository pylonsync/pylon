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
    x: field.number(),
    y: field.number(),
    z: field.number(),
    sx: field.number(),
    sy: field.number(),
    sz: field.number(),
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
    x: field.number(),
    y: field.number(),
    z: field.number(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_room_user", fields: ["roomId", "userId"], unique: true },
      { name: "by_room", fields: ["roomId"], unique: false },
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

const manifest = buildManifest({
  name: "forge",
  version: "0.1.0",
  entities: [Prim, Cursor],
  queries: [],
  actions: [],
  policies: [primPolicy, cursorPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
