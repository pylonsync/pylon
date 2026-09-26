// Pure chat logic shared by the server functions and the UI. No React, no db,
// so every rule here is covered by tests/chat.test.ts without a server.

import { siteConfig, type ChatModel } from "./site.config";

/* ------------------------------ row shapes ------------------------------ */

export type MessageRole = "user" | "assistant";

// `streaming`: the reply is being generated. `done`: finished. `stopped`: the
// user pressed Stop, content holds what they saw. `error`: the provider call
// failed, `errorCode` says why.
export type MessageStatus = "streaming" | "done" | "stopped" | "error";

export interface ConversationRow {
  id: string;
  userId: string;
  title: string;
  example?: boolean;
  createdAt: string;
  updatedAt?: string | null;
}

export interface MessageRow {
  id: string;
  conversationId: string;
  userId: string;
  role: string;
  content: string;
  status?: string | null;
  errorCode?: string | null;
  streamId?: string | null;
  model?: string | null;
  createdAt: string;
}

/* ------------------------------- models -------------------------------- */

export function findModel(id: string | null | undefined): ChatModel | undefined {
  return siteConfig.chat.models.find((m) => m.id === id);
}

export function isAllowedModel(id: string | null | undefined): id is string {
  return findModel(id) !== undefined;
}

export function modelLabel(id: string | null | undefined): string {
  return findModel(id)?.label ?? id ?? "";
}

/* ------------------------------- titles -------------------------------- */

const TITLE_MAX = 60;

// A conversation title from the first message: first non-empty line, collapsed
// whitespace, cut at a word boundary.
export function titleFromMessage(text: string): string {
  const line =
    text
      .split("\n")
      .map((l) => l.replace(/\s+/g, " ").trim())
      .find((l) => l.length > 0) ?? "";
  if (line.length === 0) return "New chat";
  if (line.length <= TITLE_MAX) return line;
  const cut = line.slice(0, TITLE_MAX);
  const space = cut.lastIndexOf(" ");
  return `${(space > 30 ? cut.slice(0, space) : cut).trimEnd()}...`;
}

export function cleanTitle(text: string): string | null {
  const t = text.replace(/\s+/g, " ").trim().slice(0, 80);
  return t.length > 0 ? t : null;
}

/* ------------------------------ history -------------------------------- */

export interface LlmTurn {
  role: MessageRole;
  content: string;
}

const HISTORY_MAX_MESSAGES = 40;
const HISTORY_MAX_CHARS = 60_000;

// The transcript sent to the model. Keeps finished and stopped replies, drops
// failed and in-flight ones, joins consecutive turns from the same role (a
// failed reply leaves two user turns in a row, which providers reject), and
// keeps the newest turns within the size caps. The result always starts with
// a user turn.
export function buildHistory(messages: MessageRow[]): LlmTurn[] {
  const turns: LlmTurn[] = [];
  for (const m of sortMessages(messages)) {
    if (m.role !== "user" && m.role !== "assistant") continue;
    if (m.role === "assistant" && m.status !== "done" && m.status !== "stopped") continue;
    const content = m.content.trim();
    if (content.length === 0) continue;
    const last = turns[turns.length - 1];
    if (last && last.role === m.role) {
      last.content = `${last.content}\n\n${content}`;
    } else {
      turns.push({ role: m.role, content });
    }
  }

  let kept = turns.slice(-HISTORY_MAX_MESSAGES);
  let chars = kept.reduce((n, t) => n + t.content.length, 0);
  while (kept.length > 1 && chars > HISTORY_MAX_CHARS) {
    chars -= kept[0].content.length;
    kept = kept.slice(1);
  }
  while (kept.length > 0 && kept[0].role !== "user") kept = kept.slice(1);
  return kept;
}

export function sortMessages<T extends { createdAt: string; id: string }>(rows: T[]): T[] {
  return [...rows].sort((a, b) =>
    a.createdAt === b.createdAt ? a.id.localeCompare(b.id) : a.createdAt < b.createdAt ? -1 : 1,
  );
}

/* ---------------------------- stream liveness ---------------------------- */

// A reply still marked `streaming` after this long belongs to a generation
// that died (server restart, crash). The UI shows it as interrupted and the
// server lets a new turn start.
export const STALE_STREAM_MS = 5 * 60 * 1000;

export function isStaleStream(m: Pick<MessageRow, "status" | "createdAt">, now: number): boolean {
  if (m.status !== "streaming") return false;
  const t = Date.parse(m.createdAt);
  return Number.isFinite(t) && now - t > STALE_STREAM_MS;
}

/* ---------------------------- sidebar groups ---------------------------- */

export interface ConversationGroup {
  label: string;
  items: ConversationRow[];
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

export function lastActivity(c: ConversationRow): string {
  return c.updatedAt || c.createdAt;
}

// Newest first, bucketed the way chat apps show history: Today, Yesterday,
// Previous 7 days, Previous 30 days, then one group per month.
export function groupConversations(rows: ConversationRow[], now: number): ConversationGroup[] {
  const today = startOfDay(now);
  const day = 24 * 60 * 60 * 1000;
  const sorted = [...rows].sort((a, b) => (lastActivity(a) < lastActivity(b) ? 1 : -1));
  const groups: ConversationGroup[] = [];
  for (const c of sorted) {
    const t = Date.parse(lastActivity(c));
    let label: string;
    if (!Number.isFinite(t) || t >= today) label = "Today";
    else if (t >= today - day) label = "Yesterday";
    else if (t >= today - 7 * day) label = "Previous 7 days";
    else if (t >= today - 30 * day) label = "Previous 30 days";
    else label = new Date(t).toLocaleDateString("en-US", { month: "long", year: "numeric" });
    const last = groups[groups.length - 1];
    if (last && last.label === label) last.items.push(c);
    else groups.push({ label, items: [c] });
  }
  return groups;
}

/* ----------------------------- rate limits ------------------------------ */

export const RATE_WINDOW_MS = 60 * 60 * 1000;

export function replyLimit(isGuest: boolean): number {
  const { guest, user } = siteConfig.chat.repliesPerHour;
  return isGuest ? guest : user;
}

export function modelAllowedFor(id: string | null | undefined, isGuest: boolean): boolean {
  if (!isAllowedModel(id)) return false;
  return !isGuest || siteConfig.chat.guestModels.includes(id);
}

/* ---------------------------- error messages ---------------------------- */

export interface ErrorCopy {
  title: string;
  body: string;
  retry: boolean;
}

// What the thread shows for a failed reply, by the error code the server
// stored on the message.
export function describeError(code: string | null | undefined): ErrorCopy {
  switch (code) {
    case "LLM_NOT_CONFIGURED":
      return {
        title: "No model provider is configured",
        body: "Add a provider key to .env and restart the server. Your message is saved.",
        retry: true,
      };
    case "MODEL_NEEDS_ACCOUNT":
      return {
        title: "That model needs an account",
        body: "Sign in to use it, or pick another model.",
        retry: true,
      };
    case "BUSY":
      return {
        title: "A reply is already being written",
        body: "Wait for it to finish, then send again.",
        retry: true,
      };
    case "MODEL_NOT_ALLOWED":
      return {
        title: "That model is not enabled",
        body: "Pick another model, or add this one to chat.models in lib/site.config.ts.",
        retry: true,
      };
    case "RATE_LIMITED":
    case "PROVIDER_HTTP_429":
      return {
        title: "Rate limit reached",
        body: "Too many replies in the last hour. Try again later.",
        retry: true,
      };
    case "PROVIDER_HTTP_401":
    case "PROVIDER_HTTP_403":
      return {
        title: "The provider rejected the API key",
        body: "Check the key in .env and restart the server.",
        retry: true,
      };
    case "PROVIDER_UNREACHABLE":
      return {
        title: "Could not reach the model provider",
        body: "Check the network connection and try again.",
        retry: true,
      };
    default:
      return {
        title: "The reply failed",
        body: "Something went wrong while generating this reply. Try again.",
        retry: true,
      };
  }
}

/* ------------------------------- guests -------------------------------- */

export function isGuestId(userId: string | null | undefined): boolean {
  return typeof userId === "string" && userId.startsWith("guest_");
}

/* ------------------------------ redirects ------------------------------ */

// A post-sign-in destination from `?next=`. Only same-site paths are
// accepted, so the parameter cannot send a user to another origin.
export function safeNextPath(next: string | null | undefined): string {
  if (!next) return "/";
  // Any control character is refused: browsers strip tabs and newlines while
  // parsing, which turns "/\t/evil.test" into "//evil.test".
  if (/[\u0000-\u001f\u007f\\]/.test(next)) return "/";
  if (!next.startsWith("/") || next.startsWith("//")) return "/";
  const base = "http://same.invalid";
  let url: URL;
  try {
    url = new URL(next, base);
  } catch {
    return "/";
  }
  if (url.origin !== base) return "/";
  return url.pathname + url.search + url.hash;
}
