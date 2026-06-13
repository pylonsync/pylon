import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// A chat message in the shared room. `authorId: field.owner()` stamps the
// signed-in (guest) user's id server-side, so an optimistic
// `db.insert("Message", { text })` can't forge the sender. The room is
// public-read — everyone in it sees every message (that's what makes it a
// chat room) — while delete is owner-only.
const Message = entity(
  "Message",
  {
    authorId: field.string().owner(),
    text: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_created", fields: ["createdAt"], unique: false }] },
);

// Public-read so the room renders for everyone; insert requires a session
// (the owner is stamped by field.owner, see the note on allowInsert); delete
// is owner-only. An entity with no policy is denied to clients by default.
const messagePolicy = policy({
  name: "message_room",
  entity: "Message",
  allowRead: "true",
  // `auth.userId != null`, not `== data.authorId`: field.owner() stamps the
  // owner AFTER the policy check, so it's null at insert-time. The stamp still
  // guarantees the message is attributed to the caller.
  allowInsert: "auth.userId != null",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.authorId",
});

// `pylon dev` serves the SSR room and the realtime API from one port. Guest
// sessions (via `<EnsureGuest>` on the page) let anyone chat with no login —
// `db.useQuery("Message")` is a live subscription, so messages appear the
// instant they're sent, in this tab or another. Natural next steps: a `Room`
// entity (+ `roomId` on Message) for multiple rooms, and a presence channel
// (`ctx.connections.*`) for a "who's here" list.
const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Message],
  queries: [],
  actions: [],
  policies: [messagePolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
