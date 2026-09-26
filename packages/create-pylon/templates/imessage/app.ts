import {
  auth,
  buildManifest,
  cron,
  discoverAppRoutes,
  discoverFunctions,
  entity,
  field,
  font,
  llm,
  policy,
} from "@pylonsync/sdk";
import { ASSISTANT_MODEL } from "./lib/assistant";

// ---------------------------------------------------------------------------
// imessage: an AI assistant people text over iMessage.
//
// Texts arrive through one of two transports (lib/transport.ts): Sendblue's
// hosted API or a relay script on your own Mac. Each inbound message is
// stored, checked against the allowlist and rate limit (lib/gate.ts), and, if
// it should be answered, handed to the `assistant` agent (functions/assistant.ts),
// which replies with conversation history and a few tools. Replies go back
// through the same transport.
//
// The dashboard is for one person: the owner, the account whose verified
// email is in PYLON_ADMIN_EMAILS. Pylon lifts that session to admin, and every
// entity below is readable by admins only. Anonymous visitors and other
// signed-in accounts read nothing.
// ---------------------------------------------------------------------------

// A person who texts the number, or whom the owner added to the allowlist.
// `handle` is normalized (lib/handles.ts): E.164 phone or lowercase email.
const Contact = entity(
  "Contact",
  {
    handle: field.string(),
    kind: field.string(),
    displayName: field.string().optional(),
    // "allowed" | "blocked" | "unknown". Only allowed contacts get answers.
    status: field.string().default("unknown"),
    // The contact texted STOP. Cleared by START.
    optedOut: field.bool().default(false),
    // When the one-time reply to an unknown sender went out.
    autoReplySentAt: field.datetime().optional(),
    // Last STOP / START confirmation sent. At most one of each per day.
    optOutReplyAt: field.datetime().optional(),
    optInReplyAt: field.datetime().optional(),
    lastMessageAt: field.datetime().optional(),
    demo: field.bool().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_handle", fields: ["handle"], unique: true },
      { name: "by_status", fields: ["status"], unique: false },
      { name: "by_created", fields: ["createdAt"], unique: false },
    ],
  },
);

// One thread per contact. It also carries the agent turn state: only one turn
// runs per conversation at a time (functions/beginTurn.ts), and texts that
// arrive during a turn are answered by the next one.
const Conversation = entity(
  "Conversation",
  {
    contactId: field.id("Contact"),
    handle: field.string(),
    lastMessageAt: field.datetime().optional(),
    lastPreview: field.string().optional(),
    lastDirection: field.string().optional(),
    // The AgentRun that holds this conversation's model transcript.
    agentRunId: field.string().optional(),
    // "idle" | "running"
    turnState: field.string().default("idle"),
    turnId: field.string().optional(),
    turnStartedAt: field.datetime().optional(),
    needsAnotherTurn: field.bool().default(false),
    lastError: field.string().optional(),
    lastErrorAt: field.datetime().optional(),
    demo: field.bool().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_contact", fields: ["contactId"], unique: true },
      { name: "by_last_message", fields: ["lastMessageAt"], unique: false },
      { name: "by_agent_run", fields: ["agentRunId"], unique: false },
    ],
  },
);

// Every text in or out.
//   direction "in":  status received | processing | answered | ignored |
//                    rate_limited | awaiting_setup | failed
//   direction "out": status queued | leased | sent | dry_run | failed
//   kind: inbound | agent | auto_reply | reminder | owner
const Message = entity(
  "Message",
  {
    conversationId: field.id("Conversation"),
    contactId: field.id("Contact"),
    direction: field.string(),
    kind: field.string(),
    text: field.string(),
    status: field.string(),
    // Why an inbound message was not answered, or why a send failed.
    detail: field.string().optional(),
    transport: field.string().optional(),
    // Transport message id ("sendblue:<handle>" / "relay:<guid>"). Unique, so
    // a retried webhook or a repeated relay batch stores nothing twice.
    externalId: field.string().optional(),
    turnId: field.string().optional(),
    agentRunId: field.string().optional(),
    // Delivery lease: a relay sync or a Sendblue delivery job holds a queued
    // reply until this time. `leaseToken` names the holder, so only it can
    // settle the message.
    leaseUntil: field.datetime().optional(),
    leaseToken: field.string().optional(),
    attempts: field.int().default(0),
    sentAt: field.datetime().optional(),
    demo: field.bool().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_conversation_created", fields: ["conversationId", "createdAt"], unique: false },
      { name: "by_contact_created", fields: ["contactId", "createdAt"], unique: false },
      { name: "by_external_id", fields: ["externalId"], unique: true },
      { name: "by_status", fields: ["status"], unique: false },
    ],
    // Messages grow forever. The dashboard replica holds the last 30 days.
    sync: { where: 'data.createdAt >= ago("30d")', limit: 5000 },
  },
);

// Short facts the assistant keeps about a contact (functions/assistant.ts tools).
const Note = entity(
  "Note",
  {
    contactId: field.id("Contact"),
    text: field.string(),
    demo: field.bool().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_contact", fields: ["contactId"], unique: false }] },
);

// A follow-up text the assistant scheduled. functions/deliverReminder.ts runs
// at `dueAt` through the job scheduler.
const Reminder = entity(
  "Reminder",
  {
    contactId: field.id("Contact"),
    conversationId: field.id("Conversation"),
    text: field.string(),
    dueAt: field.datetime(),
    // "scheduled" | "sending" | "sent" | "cancelled" | "failed"
    status: field.string(),
    detail: field.string().optional(),
    sentAt: field.datetime().optional(),
    demo: field.bool().default(false),
    createdAt: field.datetime().defaultNow(),
  },
  {
    indexes: [
      { name: "by_contact_status", fields: ["contactId", "status"], unique: false },
      { name: "by_contact_created", fields: ["contactId", "createdAt"], unique: false },
      { name: "by_due", fields: ["dueAt"], unique: false },
    ],
  },
);

// Owner settings. One row, key "main" (lib/settings.ts fills defaults).
const Settings = entity(
  "Settings",
  {
    key: field.string(),
    ownerName: field.string().optional(),
    timezone: field.string(),
    // "ignore" | "reply_once"
    unknownSenderPolicy: field.string(),
    unknownSenderReply: field.string(),
    rateLimitPerHour: field.int(),
    paused: field.bool().default(false),
    demoSeededAt: field.datetime().optional(),
    updatedAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_key", fields: ["key"], unique: true }] },
);

// Last-seen times per transport, for the dashboard's status panel.
const TransportState = entity(
  "TransportState",
  {
    name: field.string(),
    lastInboundAt: field.datetime().optional(),
    lastOutboundAt: field.datetime().optional(),
    lastError: field.string().optional(),
    lastErrorAt: field.datetime().optional(),
    relayLastSeenAt: field.datetime().optional(),
    relayVersion: field.string().optional(),
    relayHost: field.string().optional(),
  },
  { indexes: [{ name: "by_name", fields: ["name"], unique: true }] },
);

// The owner's account. Sign-in is by email code only (/login): the code
// proves the address, and Pylon's PYLON_ADMIN_EMAILS check only promotes
// verified emails. There is deliberately no `passwordHash` column, so the
// built-in password routes cannot create or sign in to an account. That
// closes a pre-registration takeover: without it, anyone could register the
// owner's email with a password before the owner first signs in, and the
// owner's later code sign-in would mark that attacker-created account
// verified, making the attacker's password an admin login.
const User = entity(
  "User",
  {
    email: field.string(),
    displayName: field.string().optional(),
    avatarColor: field.string().optional(),
    emailVerified: field.datetime().optional(),
    createdAt: field.datetime().defaultNow(),
  },
  { indexes: [{ name: "by_email", fields: ["email"], unique: true }] },
);

// Owner-only reads; no client writes. Every write goes through a function.
const ownerOnly = (name: string, entityName: string) =>
  policy({
    name,
    entity: entityName,
    allowRead: "auth.isAdmin",
    allowInsert: "false",
    allowUpdate: "false",
    allowDelete: "false",
  });

const userPolicy = policy({
  name: "user_self",
  entity: "User",
  allowRead: "auth.userId != null && auth.userId == data.id",
  allowInsert: "false",
  allowUpdate: "false",
  allowDelete: "false",
});

const fns = await discoverFunctions();

const manifest = buildManifest({
  name: "__APP_NAME__",
  version: "0.1.0",
  entities: [Contact, Conversation, Message, Note, Reminder, Settings, TransportState, User],
  queries: fns.queries,
  actions: fns.actions,
  // discoverFunctions() reports the `assistant` agent; this adds the
  // framework's AgentRun/AgentMessage entities (owner-scoped, synced).
  hasAgents: fns.hasAgents,
  policies: [
    ownerOnly("contact_owner", "Contact"),
    ownerOnly("conversation_owner", "Conversation"),
    ownerOnly("message_owner", "Message"),
    ownerOnly("note_owner", "Note"),
    ownerOnly("reminder_owner", "Reminder"),
    ownerOnly("settings_owner", "Settings"),
    ownerOnly("transport_state_owner", "TransportState"),
    userPolicy,
  ],
  auth: auth(),
  // The one model the assistant uses. ctx.llm refuses any other id.
  llm: llm({ defaultModel: ASSISTANT_MODEL, allowedModels: [ASSISTANT_MODEL] }),
  crons: [
    // Recovers turns whose worker died and relay replies whose lease expired.
    cron("*/5 * * * *", "sweep", { description: "Recover stuck turns and expired relay leases" }),
  ],
  fonts: [
    font({
      family: "Geist",
      variable: "--font-ui",
      weights: ["400", "500", "600", "700"],
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

// The CLI runs `bun run app.ts` and reads the manifest from stdout. Keep this line.
console.log(JSON.stringify(manifest, null, 2));

export default manifest;
