// Database helpers shared by the server functions. Each takes the function's
// `ctx` (mutation ctx for writes) so the function files stay thin and every
// write path (webhook, relay, agent tools, dashboard) applies the same rules.

import type { MutationCtx, QueryCtx } from "@pylonsync/functions";
import {
  DEFAULT_SETTINGS,
  decideInbound,
  mayConfirmKeyword,
  NEW_UNKNOWN_CONTACTS_PER_HOUR,
  OPT_IN_REPLY,
  OPT_OUT_REPLY,
  UNKNOWN_SENDER_STORE_LIMIT,
} from "./gate";
import type { ContactStatus, GateSettings, UnknownSenderPolicy } from "./gate";
import { normalizeHandle } from "./handles";
import { isDevMode, isDryRun, selectedTransport, type InboundMessage, type TransportName } from "./transport";

type Row = Record<string, unknown>;
type ReadCtx = Pick<QueryCtx, "db" | "env">;
type WriteCtx = Pick<MutationCtx, "db" | "env" | "scheduler">;

export interface ContactRow {
  id: string;
  handle: string;
  kind: string;
  displayName: string | null;
  status: ContactStatus;
  optedOut: boolean;
  autoReplySentAt: string | null;
  optOutReplyAt: string | null;
  optInReplyAt: string | null;
}

export interface ConversationRow {
  id: string;
  contactId: string;
  handle: string;
  agentRunId: string | null;
  turnState: string;
  turnId: string | null;
  turnStartedAt: string | null;
  needsAnotherTurn: boolean;
}

export interface SettingsRow {
  id: string | null;
  ownerName: string;
  timezone: string;
  unknownSenderPolicy: UnknownSenderPolicy;
  unknownSenderReply: string;
  rateLimitPerHour: number;
  paused: boolean;
  demoSeededAt: string | null;
}

/** A running turn older than this is treated as dead and may be taken over. */
export const TURN_STALE_MS = 10 * 60 * 1000;

const str = (v: unknown): string | null => (typeof v === "string" && v !== "" ? v : null);

export function toContact(row: Row): ContactRow {
  const status = row.status === "allowed" || row.status === "blocked" ? row.status : "unknown";
  return {
    id: String(row.id),
    handle: String(row.handle),
    kind: String(row.kind ?? "phone"),
    displayName: str(row.displayName),
    status,
    optedOut: row.optedOut === true || row.optedOut === 1,
    autoReplySentAt: str(row.autoReplySentAt),
    optOutReplyAt: str(row.optOutReplyAt),
    optInReplyAt: str(row.optInReplyAt),
  };
}

export function toConversation(row: Row): ConversationRow {
  return {
    id: String(row.id),
    contactId: String(row.contactId),
    handle: String(row.handle),
    agentRunId: str(row.agentRunId),
    turnState: String(row.turnState ?? "idle"),
    turnId: str(row.turnId),
    turnStartedAt: str(row.turnStartedAt),
    needsAnotherTurn: row.needsAnotherTurn === true || row.needsAnotherTurn === 1,
  };
}

export async function loadSettings(ctx: ReadCtx): Promise<SettingsRow> {
  const row = await ctx.db.lookup("Settings", "key", "main");
  const policy = row?.unknownSenderPolicy === "reply_once" ? "reply_once" : "ignore";
  const rate = Number(row?.rateLimitPerHour);
  return {
    id: row ? String(row.id) : null,
    ownerName: str(row?.ownerName) ?? DEFAULT_SETTINGS.ownerName,
    timezone: str(row?.timezone) ?? DEFAULT_SETTINGS.timezone,
    unknownSenderPolicy: row ? policy : DEFAULT_SETTINGS.unknownSenderPolicy,
    unknownSenderReply: str(row?.unknownSenderReply) ?? DEFAULT_SETTINGS.unknownSenderReply,
    rateLimitPerHour: Number.isFinite(rate) && rate >= 0 ? rate : DEFAULT_SETTINGS.rateLimitPerHour,
    paused: row?.paused === true || row?.paused === 1,
    demoSeededAt: str(row?.demoSeededAt),
  };
}

export function gateSettings(s: SettingsRow): GateSettings {
  return { unknownSenderPolicy: s.unknownSenderPolicy, rateLimitPerHour: s.rateLimitPerHour, paused: s.paused };
}

/** The Settings row, created with defaults on first use. */
export async function ensureSettings(ctx: WriteCtx): Promise<SettingsRow> {
  const s = await loadSettings(ctx);
  if (s.id) return s;
  const id = await ctx.db.insert("Settings", {
    key: "main",
    ownerName: DEFAULT_SETTINGS.ownerName,
    timezone: DEFAULT_SETTINGS.timezone,
    unknownSenderPolicy: DEFAULT_SETTINGS.unknownSenderPolicy,
    unknownSenderReply: DEFAULT_SETTINGS.unknownSenderReply,
    rateLimitPerHour: DEFAULT_SETTINGS.rateLimitPerHour,
    paused: false,
    updatedAt: new Date().toISOString(),
  });
  return { ...s, id };
}

export async function getOrCreateContact(
  ctx: WriteCtx,
  handle: string,
  defaults: { status?: ContactStatus; displayName?: string | null; demo?: boolean } = {},
): Promise<ContactRow> {
  const existing = await ctx.db.lookup("Contact", "handle", handle);
  if (existing) return toContact(existing);
  const kind = normalizeHandle(handle)?.kind ?? "phone";
  const now = new Date().toISOString();
  const row = {
    handle,
    kind,
    displayName: defaults.displayName ?? null,
    status: defaults.status ?? "unknown",
    optedOut: false,
    demo: defaults.demo ?? false,
    createdAt: now,
  };
  const id = await ctx.db.insert("Contact", row);
  return toContact({ id, ...row });
}

export async function getOrCreateConversation(
  ctx: WriteCtx,
  contact: ContactRow,
  demo = false,
): Promise<ConversationRow> {
  const existing = await ctx.db.lookup("Conversation", "contactId", contact.id);
  if (existing) return toConversation(existing);
  const row = {
    contactId: contact.id,
    handle: contact.handle,
    turnState: "idle",
    needsAnotherTurn: false,
    demo,
    createdAt: new Date().toISOString(),
  };
  const id = await ctx.db.insert("Conversation", row);
  return toConversation({ id, ...row });
}

export async function touchTransport(
  ctx: WriteCtx,
  name: TransportName,
  patch: Record<string, unknown>,
): Promise<void> {
  const row = await ctx.db.lookup("TransportState", "name", name);
  if (row) await ctx.db.update("TransportState", String(row.id), patch);
  else await ctx.db.insert("TransportState", { name, ...patch });
}

function preview(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > 120 ? `${flat.slice(0, 119)}…` : flat;
}

async function bumpConversation(
  ctx: WriteCtx,
  conversationId: string,
  contactId: string,
  text: string,
  direction: "in" | "out",
  at: string,
): Promise<void> {
  await ctx.db.update("Conversation", conversationId, {
    lastMessageAt: at,
    lastPreview: preview(text),
    lastDirection: direction,
  });
  await ctx.db.update("Contact", contactId, { lastMessageAt: at });
}

/** Inbound messages from this contact in the last hour that were (or are being) answered. */
async function answeredInLastHour(ctx: ReadCtx, contactId: string): Promise<{ answered: number; stored: number }> {
  const since = new Date(Date.now() - 60 * 60 * 1000).toISOString();
  const rows = await ctx.db.query("Message", {
    contactId,
    createdAt: { $gte: since },
  });
  let answered = 0;
  let stored = 0;
  for (const r of rows) {
    if (r.direction !== "in") continue;
    stored += 1;
    // STOP/START count too: each one can trigger a confirmation text.
    if (
      r.status === "received" ||
      r.status === "processing" ||
      r.status === "answered" ||
      r.detail === "opt_out" ||
      r.detail === "opt_in"
    ) {
      answered += 1;
    }
  }
  return { answered, stored };
}

export interface QueueOutboundInput {
  conversationId: string;
  contactId: string;
  handle: string;
  text: string;
  kind: "agent" | "auto_reply" | "reminder" | "owner";
  turnId?: string | null;
  agentRunId?: string | null;
}

/**
 * Store an outbound message and arrange delivery.
 *   sendblue → status "queued", and a deliverOutbound job sends the
 *              conversation's queued messages in order.
 *   relay    → status "queued"; the relay collects it on its next sync.
 *   dry run  → status "dry_run"; nothing is sent.
 *   none     → status "failed" with the reason.
 */
export async function queueOutbound(ctx: WriteCtx, input: QueueOutboundInput): Promise<string> {
  const transport = selectedTransport(ctx.env);
  const now = new Date().toISOString();
  let status = "queued";
  let detail: string | null = null;
  if (!transport) {
    status = "failed";
    detail = "No transport configured (IMESSAGE_TRANSPORT).";
  } else if (isDryRun(ctx.env)) {
    status = "dry_run";
    detail = isDevMode(ctx.env)
      ? "Development server: nothing was sent (set IMESSAGE_ALLOW_DEV_SENDS=1 to send)."
      : "IMESSAGE_DRY_RUN is on; nothing was sent.";
  }
  const id = await ctx.db.insert("Message", {
    conversationId: input.conversationId,
    contactId: input.contactId,
    direction: "out",
    kind: input.kind,
    text: input.text,
    status,
    detail,
    transport: transport ?? null,
    turnId: input.turnId ?? null,
    agentRunId: input.agentRunId ?? null,
    attempts: 0,
    sentAt: status === "dry_run" ? now : null,
    createdAt: now,
  });
  await bumpConversation(ctx, input.conversationId, input.contactId, input.text, "out", now);
  if (status === "queued" && transport === "sendblue") {
    await ctx.scheduler.runAfter(0, "deliverOutbound", { conversationId: input.conversationId });
  }
  return id;
}

export type IngestStatus =
  | "duplicate"
  | "dropped"
  | "answer"
  | "auto_reply"
  | "opt_out"
  | "opt_in"
  | "ignored";

/**
 * Store one inbound message and act on the gate's decision. Idempotent on
 * `message.externalId`.
 */
export async function ingestInbound(
  ctx: WriteCtx,
  message: InboundMessage,
  transport: TransportName,
): Promise<{ status: IngestStatus; messageId?: string }> {
  const handle = normalizeHandle(message.handle)?.handle;
  if (!handle) return { status: "dropped" };
  if (await ctx.db.lookup("Message", "externalId", message.externalId)) return { status: "duplicate" };

  const existingContact = await ctx.db.lookup("Contact", "handle", handle);
  if (!existingContact) {
    // A burst of new senders (or a leaked relay token inventing handles)
    // cannot create contacts without bound.
    const since = new Date(Date.now() - 60 * 60 * 1000).toISOString();
    const recent = await ctx.db.query("Contact", {
      createdAt: { $gte: since },
      $limit: NEW_UNKNOWN_CONTACTS_PER_HOUR + 1,
    });
    if (recent.filter((c) => c.status === "unknown").length >= NEW_UNKNOWN_CONTACTS_PER_HOUR) {
      return { status: "dropped" };
    }
  }
  const contact = existingContact ? toContact(existingContact) : await getOrCreateContact(ctx, handle);
  const counts = await answeredInLastHour(ctx, contact.id);
  // A sender who is not on the allowlist cannot fill the database: past the
  // hourly cap their texts are not stored at all.
  if (contact.status !== "allowed" && counts.stored >= UNKNOWN_SENDER_STORE_LIMIT) {
    return { status: "dropped" };
  }
  const conversation = await getOrCreateConversation(ctx, contact);
  const settings = await ensureSettings(ctx);
  const decision = decideInbound(contact, message.text, gateSettings(settings), counts.answered);

  const now = new Date().toISOString();
  let status = "received";
  let detail: string | null = null;
  if (decision.action === "ignore") {
    status = decision.reason === "rate_limited" ? "rate_limited" : "ignored";
    detail = decision.reason;
  } else if (decision.action === "auto_reply") {
    status = "ignored";
    detail = "unknown_sender";
  } else if (decision.action === "opt_out" || decision.action === "opt_in") {
    status = "ignored";
    detail = decision.action;
  }

  const messageId = await ctx.db.insert("Message", {
    conversationId: conversation.id,
    contactId: contact.id,
    direction: "in",
    kind: "inbound",
    text: message.text,
    status,
    detail,
    transport,
    externalId: message.externalId,
    attempts: 0,
    sentAt: message.sentAt,
    createdAt: now,
  });
  await bumpConversation(ctx, conversation.id, contact.id, message.text, "in", now);
  await touchTransport(ctx, transport, { lastInboundAt: now });

  const reply = (text: string, kind: QueueOutboundInput["kind"]) =>
    queueOutbound(ctx, {
      conversationId: conversation.id,
      contactId: contact.id,
      handle: contact.handle,
      text,
      kind,
    });

  switch (decision.action) {
    case "answer":
      await ctx.scheduler.runAfter(0, "processTurn", { conversationId: conversation.id });
      break;
    case "auto_reply":
      // Claimed in the same transaction as the insert, so two concurrent
      // texts from a stranger produce one reply.
      await ctx.db.update("Contact", contact.id, { autoReplySentAt: now });
      await reply(settings.unknownSenderReply, "auto_reply");
      break;
    case "opt_out": {
      // Confirm only to someone this number has texted, at most once a day,
      // so alternating STOP and START cannot make the number send in a loop.
      const wasContacted = contact.status === "allowed" || contact.autoReplySentAt !== null;
      const confirm = wasContacted && !contact.optedOut && mayConfirmKeyword(contact.optOutReplyAt, new Date(now));
      await ctx.db.update("Contact", contact.id, { optedOut: true, ...(confirm ? { optOutReplyAt: now } : {}) });
      if (confirm) await reply(OPT_OUT_REPLY, "auto_reply");
      break;
    }
    case "opt_in": {
      const confirm = contact.status === "allowed" && mayConfirmKeyword(contact.optInReplyAt, new Date(now));
      await ctx.db.update("Contact", contact.id, { optedOut: false, ...(confirm ? { optInReplyAt: now } : {}) });
      if (confirm) await reply(OPT_IN_REPLY, "auto_reply");
      break;
    }
    case "ignore":
      break;
  }
  return { status: decision.action === "ignore" ? "ignored" : decision.action, messageId };
}

/** The owner's user id: the first verified account whose email is in PYLON_ADMIN_EMAILS. */
export async function findOwner(ctx: ReadCtx): Promise<{ userId: string; email: string } | null> {
  const emails = (ctx.env.PYLON_ADMIN_EMAILS ?? "")
    .split(",")
    .map((e) => e.trim().toLowerCase())
    .filter(Boolean);
  for (const email of emails) {
    const user = await ctx.db.lookup("User", "email", email);
    if (user && str(user.emailVerified)) return { userId: String(user.id), email };
  }
  return null;
}
