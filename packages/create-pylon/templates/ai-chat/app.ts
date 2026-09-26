import {
  entity,
  field,
  policy,
  auth,
  llm,
  buildManifest,
  discoverAppRoutes,
  discoverFunctions,
  font,
} from "@pylonsync/sdk";
import { siteConfig } from "./lib/site.config";

// ---------------------------------------------------------------------------
// ai-chat: a streaming AI assistant. Every visitor gets a guest session and can
// chat right away; signing in moves the guest's conversations to the account.
//
//   Conversation  a chat thread. Owner-scoped.
//   Message       one turn. Assistant replies are written by the `respond`
//                 function, which streams tokens from `ctx.llm.stream` and
//                 records the resumable stream id on the row, so any other tab
//                 can attach to the same reply while it generates.
//   ReplyLog      one row per generation, for limits and the busy check.
//   GuestClaim    a one-time code that lets a guest's history move to the
//                 account they sign in to. Never readable by clients.
//   User          the account (email/password is built in).
// ---------------------------------------------------------------------------

const Conversation = entity(
  "Conversation",
  {
    userId: field.string().owner(),
    title: field.string().default("New chat"),
    // Set on the seeded examples shown when no provider key is configured.
    example: field.boolean().default(false),
    createdAt: field.datetime().defaultNow(),
    updatedAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_updated", fields: ["updatedAt"], unique: false },
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
    // "streaming" | "done" | "stopped" | "error"; see lib/chat.ts.
    status: field.string().default("done"),
    errorCode: field.string().optional(),
    // Resumable stream id of the reply while it generates.
    streamId: field.string().optional(),
    model: field.string().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_conversation", fields: ["conversationId"], unique: false },
      { name: "by_user", fields: ["userId"], unique: false },
    ],
  },
);

// One row per generation, never deleted by regenerate or conversation
// delete. The hourly limits count these rows, and `finishedAt` stays null
// while the provider call runs, so one user cannot run two generations at
// once. Never readable by clients.
const ReplyLog = entity(
  "ReplyLog",
  {
    userId: field.string(),
    messageId: field.string(),
    isGuest: field.boolean(),
    model: field.string(),
    createdAt: field.datetime(),
    finishedAt: field.datetime().optional(),
  },
  {
    indexes: [
      { name: "by_user", fields: ["userId"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
      { name: "by_message", fields: ["messageId"], unique: false },
    ],
  },
);

const GuestClaim = entity(
  "GuestClaim",
  {
    guestId: field.string(),
    codeHash: field.string().serverOnly(),
    expiresAt: field.datetime(),
  },
  { indexes: [{ name: "by_code", fields: ["codeHash"], unique: true }] },
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

// Clients read their own rows. Every write goes through a server function:
// `respond` writes messages, `createConversation`, `renameConversation`, and
// `deleteConversation` manage threads, and `stopResponse` ends a reply. That
// keeps a client from writing assistant turns, moving a message to another
// thread, or setting fields such as `example`.
const conversationPolicy = policy({
  name: "conversation_owner",
  entity: "Conversation",
  allowRead: "auth.userId != null && auth.userId == data.userId",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const messagePolicy = policy({
  name: "message_owner",
  entity: "Message",
  allowRead: "auth.userId != null && auth.userId == data.userId",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const replyLogPolicy = policy({
  name: "reply_log_server_only",
  entity: "ReplyLog",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const guestClaimPolicy = policy({
  name: "guest_claim_server_only",
  entity: "GuestClaim",
  allowRead: "false",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Conversation, Message, ReplyLog, GuestClaim, User],
  queries: fns.queries,
  actions: fns.actions,
  policies: [conversationPolicy, messagePolicy, replyLogPolicy, guestClaimPolicy, userPolicy],
  auth: auth(),
  // The model list in lib/site.config.ts is the allowlist: ctx.llm refuses any
  // other model id.
  llm: llm({
    defaultModel: siteConfig.chat.defaultModel,
    allowedModels: siteConfig.chat.models.map((m) => m.id),
  }),
  // Self-hosted fonts: the build downloads the woff2 files, serves them from
  // this origin, and adds size-adjusted fallbacks so text does not shift when
  // they load. globals.css maps them to Tailwind's font-sans, font-serif, and font-mono.
  fonts: [
    font({
      family: "Geist",
      variable: "--font-ui",
      weights: ["400", "500", "600"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
    font({
      family: "Source Serif 4",
      variable: "--font-reading",
      weights: ["400", "600"],
      subsets: ["latin"],
      display: "swap",
      preload: true,
    }),
    font({
      family: "Geist Mono",
      variable: "--font-code",
      weights: ["400", "500"],
      subsets: ["latin"],
      display: "swap",
    }),
  ],
  routes: await discoverAppRoutes(),
});

// Not a debug leftover: the CLI runs `bun run app.ts` and parses stdout as
// your manifest. Deleting this line breaks every pylon command that reads your
// schema.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
