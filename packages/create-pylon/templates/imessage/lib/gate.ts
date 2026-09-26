// Who the assistant answers. Every inbound message goes through `decideInbound`
// before anything is scheduled, so the assistant never becomes a relay for
// whoever happens to text the number.
//
//   allowed contact      → answer (subject to the per-contact rate limit)
//   blocked contact      → store, never answer
//   unknown sender       → store as a request; optionally send ONE fixed reply
//   STOP / START         → opt out / back in; applies to every contact

export type ContactStatus = "allowed" | "blocked" | "unknown";
export type UnknownSenderPolicy = "ignore" | "reply_once";

export interface GateSettings {
  unknownSenderPolicy: UnknownSenderPolicy;
  /** Inbound messages per contact per rolling hour that get an answer. */
  rateLimitPerHour: number;
  /** When true the assistant answers nobody. Messages are still stored. */
  paused: boolean;
}

export interface GateContact {
  status: ContactStatus;
  optedOut: boolean;
  /** Set once the one-time reply to an unknown sender has been sent. */
  autoReplySentAt: string | null;
}

export type InboundDecision =
  | { action: "answer" }
  | { action: "auto_reply" }
  | { action: "opt_out" }
  | { action: "opt_in" }
  | {
      action: "ignore";
      reason: "blocked" | "unknown_sender" | "opted_out" | "rate_limited" | "paused";
    };

const STOP_WORDS = new Set(["stop", "stopall", "unsubscribe", "cancel", "end", "quit"]);
const START_WORDS = new Set(["start", "unstop", "yes"]);

function keyword(text: string): string {
  return text.trim().toLowerCase().replace(/[.!\s]+$/g, "");
}

export function isStopKeyword(text: string): boolean {
  return STOP_WORDS.has(keyword(text));
}

export function isStartKeyword(text: string): boolean {
  return START_WORDS.has(keyword(text));
}

export function decideInbound(
  contact: GateContact,
  text: string,
  settings: GateSettings,
  answeredInLastHour: number,
): InboundDecision {
  if (isStopKeyword(text)) return { action: "opt_out" };
  if (contact.optedOut) {
    return isStartKeyword(text) ? { action: "opt_in" } : { action: "ignore", reason: "opted_out" };
  }
  if (contact.status === "blocked") return { action: "ignore", reason: "blocked" };
  if (contact.status !== "allowed") {
    if (settings.unknownSenderPolicy === "reply_once" && !contact.autoReplySentAt && !settings.paused) {
      return { action: "auto_reply" };
    }
    return { action: "ignore", reason: "unknown_sender" };
  }
  if (settings.paused) return { action: "ignore", reason: "paused" };
  if (answeredInLastHour >= Math.max(0, settings.rateLimitPerHour)) {
    return { action: "ignore", reason: "rate_limited" };
  }
  return { action: "answer" };
}

/** Inbound rows stored per unknown sender per hour. Past this, retries and floods are dropped. */
export const UNKNOWN_SENDER_STORE_LIMIT = 20;

/** New unknown contacts created per rolling hour, across all senders. Past this, new senders are dropped. */
export const NEW_UNKNOWN_CONTACTS_PER_HOUR = 50;

/** Minimum time between two STOP (or two START) confirmations to one contact. */
export const KEYWORD_REPLY_INTERVAL_MS = 24 * 60 * 60 * 1000;

/** True when a keyword confirmation may be sent now, given when the last one went out. */
export function mayConfirmKeyword(lastSentAt: string | null, now: Date): boolean {
  if (!lastSentAt) return true;
  const last = Date.parse(lastSentAt);
  return !Number.isFinite(last) || now.getTime() - last >= KEYWORD_REPLY_INTERVAL_MS;
}

export const DEFAULT_SETTINGS = {
  ownerName: "",
  timezone: "America/Chicago",
  unknownSenderPolicy: "ignore" as UnknownSenderPolicy,
  unknownSenderReply:
    "Hi, this number is an automated assistant that only replies to people on its contact list. Your message was passed along.",
  rateLimitPerHour: 30,
  paused: false,
};

export const OPT_OUT_REPLY = "You won't get more messages from this number. Text START to resume.";
export const OPT_IN_REPLY = "You're back on. Text STOP any time to stop messages.";
