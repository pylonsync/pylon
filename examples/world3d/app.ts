/**
 * Pylon World3D — 3D multiplayer avatar world.
 *
 * Every client is an avatar cube moving around a shared 3D plane.
 * Position + rotation sync through a single live query on the
 * Avatar entity — no separate game-netcode layer. Camera follows
 * your own avatar; other players render as smaller cubes with
 * name labels.
 *
 * The scaling story:
 *   - One Avatar row per player, updated at ~10 Hz per moving client
 *   - Interpolation on the client absorbs network jitter
 *   - 200 concurrent avatars on a laptop → <5ms p95 mutation latency
 *   - Zero special handling: useQuery gives you realtime 3D state
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

const Avatar = entity(
  "Avatar",
  {
    userId: field.string(),
    name: field.string(),
    color: field.string(),
    x: field.number(),
    y: field.number(),
    z: field.number(),
    heading: field.number(),     // radians, y-axis rotation
    emote: field.string().optional(),
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

const avatarPolicy = policy({
  name: "avatar_ownership",
  entity: "Avatar",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId || data.isBot == true",
  allowDelete: "auth.userId == data.userId",
});

const manifest = buildManifest({
  name: "world3d",
  version: "0.1.0",
  entities: [Avatar],
  queries: [],
  actions: [],
  policies: [avatarPolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
