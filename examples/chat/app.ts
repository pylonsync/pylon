import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// ---------------------------------------------------------------------------
// Schema
//
// Intentionally single-tenant — this demo drops `tenantId` fields so it
// works without a workspace-selection flow. Real multi-tenant apps re-add
// tenantId everywhere the tenant_scope plugin should stamp/enforce; see
// docs/ops/SIZING.md for the scaling story.
// ---------------------------------------------------------------------------

const User = entity("User", {
  email: field.string().unique(),
  displayName: field.string(),
  avatarColor: field.string(),
  // Argon2id PHC string. Set by /api/auth/password/register; consumed
  // by /api/auth/password/login. Never exposed to the client — the
  // codegen'd type stays present, but read policies on User filter it
  // out for non-self queries.
  passwordHash: field.string(),
  createdAt: field.datetime(),
});

const Channel = entity(
  "Channel",
  {
    name: field.string().unique(),
    // Topic is the per-channel collaborative-editing demo target.
    // Two browser tabs editing the same channel's topic converge
    // through Loro's text CRDT — concurrent edits to disjoint
    // regions both land. The chat header surfaces this via the
    // useCollabText hook from @pylonsync/loro.
    topic: field.string().crdt("text").optional(),
    isPrivate: field.bool(),
    createdBy: field.id("User"),
    createdAt: field.datetime(),
  },
);

const Membership = entity(
  "Membership",
  {
    channelId: field.id("Channel"),
    userId: field.id("User"),
    role: field.string(),
    joinedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_channel_user", fields: ["channelId", "userId"], unique: true },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

const Message = entity(
  "Message",
  {
    channelId: field.id("Channel"),
    authorId: field.id("User"),
    parentMessageId: field.id("Message").optional(),
    // Body is collaborative — concurrent edits from two browser tabs
    // converge through Loro's text CRDT instead of LWW. The server
    // broadcasts a binary Loro snapshot on every write; the client's
    // useLoroDoc hook keeps the rendered text in lockstep with the
    // CRDT state.
    body: field.string().crdt("text"),
    editedAt: field.datetime().optional(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_channel_created", fields: ["channelId", "createdAt"], unique: false },
      { name: "by_author", fields: ["authorId"], unique: false },
      { name: "by_parent", fields: ["parentMessageId"], unique: false },
    ],
  },
);

const Reaction = entity(
  "Reaction",
  {
    messageId: field.id("Message"),
    userId: field.id("User"),
    emoji: field.string(),
    createdAt: field.datetime(),
  },
  {
    indexes: [
      {
        name: "by_message_user_emoji",
        fields: ["messageId", "userId", "emoji"],
        unique: true,
      },
    ],
  },
);

const ReadMarker = entity(
  "ReadMarker",
  {
    userId: field.id("User"),
    channelId: field.id("Channel"),
    lastReadAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_user_channel", fields: ["userId", "channelId"], unique: true },
    ],
  },
);

// ---------------------------------------------------------------------------
// Policies — authenticated-user-only on writes; no multi-tenant isolation
// in this demo. Real apps layer on `data.tenantId == auth.tenantId` here.
// ---------------------------------------------------------------------------

// Ownership-aware policies. Reads are open to any authenticated user;
// mutations require the caller to own the row (data.<owner> == auth.userId).
// The raw /api/entities PATCH/DELETE endpoints consult these directly, so
// it's no longer possible to edit or delete someone else's message by
// poking the raw URL. Server functions still provide extra validation but
// are no longer the sole line of defense.

const channelPolicy = policy({
  name: "channel_ownership",
  entity: "Channel",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.createdBy",
  allowDelete: "auth.userId == data.createdBy",
});

const messagePolicy = policy({
  name: "message_ownership",
  entity: "Message",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId == data.authorId",
  allowDelete: "auth.userId == data.authorId",
});

const reactionPolicy = policy({
  name: "reaction_ownership",
  entity: "Reaction",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowDelete: "auth.userId == data.userId",
});

const membershipPolicy = policy({
  name: "membership_ownership",
  entity: "Membership",
  allowRead: "auth.userId != null",
  allowInsert: "auth.userId != null",
  allowDelete: "auth.userId == data.userId",
});

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

const manifest = buildManifest({
  name: "chat",
  version: "0.1.0",
  entities: [User, Channel, Membership, Message, Reaction, ReadMarker],
  queries: [],
  actions: [],
  policies: [
    channelPolicy,
    messagePolicy,
    reactionPolicy,
    membershipPolicy,
  ],
  routes: [],
});

// Emit canonical manifest JSON to stdout.
// Used by: pylon codegen examples/chat/app.ts
console.log(JSON.stringify(manifest, null, 2));
