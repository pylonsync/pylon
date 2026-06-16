import {
  entity,
  field,
  policy,
  auth,
  buildManifest,
  discoverAppRoutes,
} from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// ai-chat — a streaming AI chat app. Tokens stream from the built-in
// `POST /api/ai/stream` endpoint (your PYLON_AI_API_KEY never leaves the
// server); the conversation itself is sync-backed, so your chats follow you
// across tabs and devices in realtime — open two tabs and a reply you send in
// one shows up in the other as it's saved.
//
// Two data entities (+ User):
//   • Conversation — a chat thread. Owner-scoped: you only ever see your own.
//   • Message      — one turn (user or assistant). Owner-scoped too. Both use
//                    `field.owner()` on userId, so the client can insert with a
//                    plain optimistic `db.insert` and the id is stamped from the
//                    session — unspoofable, no server function needed to write.
//   • User         — the account (email/password is built in).
//
// There's no single "owner"/admin — it's multi-user: every signed-in (or guest)
// visitor gets their own private chats.
// ---------------------------------------------------------------------------

const Conversation = entity(
  "Conversation",
  {
    userId: field.string().owner(),
    title: field.string().default("New chat"),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
);

const Message = entity(
  "Message",
  {
    conversationId: field.string(),
    userId: field.string().owner(),
    role: field.string(), // "user" | "assistant"
    content: field.string(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_conversation", fields: ["conversationId"], unique: false },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    passwordHash: field.string().serverOnly().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// Conversations + messages are PRIVATE: a signed-in (or guest) user can only
// read, create, and modify their OWN rows. `field.owner()` stamps userId from
// the session on insert, so "create your own" is enforced at write time and
// reads are scoped by the same id — your chats never leak to anyone else, and
// the sync engine only ever ships you yours.
const conversationPolicy = policy({
  name: "conversation_owner",
  entity: "Conversation",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const messagePolicy = policy({
  name: "message_owner",
  entity: "Message",
  allowRead: "auth.userId == data.userId",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.userId",
  allowDelete: "auth.userId == data.userId",
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Conversation, Message, User],
  // No custom functions needed: messages are written with optimistic db.insert
  // (owner-stamped), and streaming uses the built-in POST /api/ai/stream.
  queries: [],
  actions: [],
  policies: [conversationPolicy, messagePolicy, userPolicy],
  auth: auth(),
  routes: await discoverAppRoutes(),
});

console.log(JSON.stringify(manifest, null, 2));

export default manifest;
