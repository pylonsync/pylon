import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// A chat message in the shared room. `authorId: field.owner()` stamps the
// signed-in (guest) user's id server-side, so an optimistic
// `db.insert("Message", { text })` can't forge the sender.
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

// buildManifest assembles the entities, policies, auth, and routes into the
// single manifest `pylon dev` serves — SSR room + realtime API on one port.
// Every non-internal function in functions/ becomes a manifest entry —
// this is what makes them show up in /api/manifest, the OpenAPI spec,
// and `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Message],
  queries: fns.queries,
  actions: fns.actions,
  policies: [messagePolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
