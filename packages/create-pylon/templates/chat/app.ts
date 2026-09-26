import {
  entity,
  field,
  policy,
  auth,
  font,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
} from "@pylonsync/sdk";

// Data model
//
//   • Channel: a room in the sidebar (#general, #launch, …). Created by the
//              seed; clients can read but not write.
//   • Member:  a display name + avatar hue per user, including guests. Written
//              only by the joinChat / renameMember functions, which take the
//              user id from the session.
//   • Message: one chat line. `authorId: field.owner()` stamps the sender from
//              the session on the server, so the optimistic
//              `db.insert("Message", …)` in the client cannot forge it.
//
// Presence ("3 online") and typing indicators are not stored. They ride on a
// presence room (`useRoom` in app/chat-client.tsx).
const Channel = entity(
  "Channel",
  {
    slug: field.string().unique(),
    name: field.string(),
    topic: field.string(),
    position: field.int(),
  },
  { indexes: [{ name: "by_position", fields: ["position"], unique: false }] },
);

const Member = entity("Member", {
  userId: field.string().unique(),
  name: field.string(),
  // Avatar color, as an OKLCH hue angle (0–359).
  hue: field.int(),
  createdAt: field.datetime().defaultNow(),
});

const Message = entity(
  "Message",
  {
    channelId: field.string(),
    authorId: field.string().owner(),
    text: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_channel_created", fields: ["channelId", "createdAt"], unique: false },
    ],
  },
);

// Everyone can read channels. Only the seed in functions/joinChat.ts writes them.
const channelPolicy = policy({
  name: "channel_public_read",
  entity: "Channel",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// Names and avatar hues are public so every message can show its author.
// Writes go through joinChat / renameMember, which validate the name and use
// the caller's session id.
const memberPolicy = policy({
  name: "member_public_read",
  entity: "Member",
  allowRead: "true",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

// The room is public-read. Insert needs a session (guests count), a channel
// that exists, and no client-supplied `createdAt`: the server stamps the time,
// so a client cannot pin its message to the bottom with a future date.
// `auth.userId != null` rather than `== data.authorId`: field.owner() stamps
// the owner after the policy check, and rejects any other user's id.
// Messages can't be edited; the author can delete their own.
const messagePolicy = policy({
  name: "message_room",
  entity: "Message",
  allowRead: "true",
  allowInsert:
    "auth.userId != null && data.createdAt == null && exists(Channel where id == data.channelId)",
  allowUpdate: "false",
  allowDelete: "auth.userId == data.authorId",
});

// buildManifest assembles the entities, policies, auth, and routes into the
// single manifest `pylon dev` serves: SSR page + realtime API on one port.
// Every non-internal function in functions/ becomes a manifest entry, which
// is what makes them show up in /api/manifest, the OpenAPI spec, and
// `pylon codegen`.
const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Channel, Member, Message],
  queries: fns.queries,
  actions: fns.actions,
  policies: [channelPolicy, memberPolicy, messagePolicy],
  auth: auth(),
  // Self-hosted Geist: the build fetches the woff2, serves it same-origin,
  // preloads it, and adds a size-adjusted fallback face so text does not
  // shift when it loads. globals.css reads the variables.
  fonts: [
    font({
      family: "Geist",
      variable: "--font-geist",
      weights: ["400", "500", "600"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
    font({
      family: "Geist Mono",
      variable: "--font-geist-mono",
      weights: ["400", "500"],
      subsets: ["latin"],
      display: "swap",
      preload: false,
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest (see crates/cli/src/bun.rs). Deleting this line breaks every
// pylon command that reads your schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
