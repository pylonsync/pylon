// Pure display helpers for the dashboard. No React, no data access.

export interface ThreadMessage {
  id: string;
  direction: "in" | "out";
  kind: string;
  text: string;
  status: string;
  detail?: string | null;
  createdAt: string;
}

export function initials(name: string | null | undefined, handle: string): string {
  const source = (name ?? "").trim();
  if (source) {
    const parts = source.split(/\s+/).filter(Boolean);
    const letters = parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : parts[0].slice(0, 2);
    return letters.toUpperCase();
  }
  if (handle.includes("@")) return handle[0].toUpperCase();
  return handle.slice(-2);
}

/** "3:04 PM" today, "Yesterday", weekday within a week, else "Sep 3". */
export function listTime(iso: string | null | undefined, now = new Date()): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
  if (days === 1) return "Yesterday";
  if (days < 7) return d.toLocaleDateString("en-US", { weekday: "long" });
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

/** Separator label between bubbles, iMessage style: "Today 3:04 PM". */
export function separatorLabel(iso: string, now = new Date()): string {
  const d = new Date(iso);
  const time = d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
  const day = listTime(iso, now);
  return day.includes(":") ? `Today ${time}` : `${day} ${time}`;
}

/** "just now", "2 min ago", "3 h ago", or a date. */
export function ago(iso: string | null | undefined, now = new Date()): string {
  if (!iso) return "never";
  const ms = now.getTime() - new Date(iso).getTime();
  if (!Number.isFinite(ms)) return "never";
  if (ms < 45_000) return "just now";
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)} min ago`;
  if (ms < 86_400_000) return `${Math.round(ms / 3_600_000)} h ago`;
  return new Date(iso).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

export interface ThreadItem {
  message: ThreadMessage;
  /** Show a time separator above this bubble. */
  separator: string | null;
  /** Last bubble in a run from the same side: gets the tail. */
  tail: boolean;
  /** Caption under the bubble (status), or null. */
  caption: string | null;
  captionTone: "muted" | "warn" | "error";
}

const GAP_FOR_SEPARATOR_MS = 60 * 60 * 1000;

/** Caption for a message whose status the owner should see. */
export function statusCaption(m: ThreadMessage, isLastOutbound: boolean): Pick<ThreadItem, "caption" | "captionTone"> {
  if (m.direction === "in") {
    switch (m.status) {
      case "awaiting_setup":
        return { caption: "Not answered yet: waiting for setup", captionTone: "warn" };
      case "rate_limited":
        return { caption: "Not answered: hourly limit reached", captionTone: "warn" };
      case "failed":
        return { caption: `Reply failed${m.detail ? `: ${m.detail}` : ""}`, captionTone: "error" };
      case "ignored":
        if (m.detail === "unknown_sender") return { caption: "Not answered: not on your allowlist", captionTone: "muted" };
        if (m.detail === "blocked") return { caption: "Not answered: blocked", captionTone: "muted" };
        if (m.detail === "opt_out") return { caption: "Texted STOP: opted out", captionTone: "muted" };
        if (m.detail === "opt_in") return { caption: "Texted START: opted back in", captionTone: "muted" };
        if (m.detail === "opted_out") return { caption: "Not answered: opted out", captionTone: "muted" };
        if (m.detail === "paused") return { caption: "Not answered: assistant paused", captionTone: "muted" };
        return { caption: "Not answered", captionTone: "muted" };
      default:
        return { caption: null, captionTone: "muted" };
    }
  }
  const who = m.kind === "owner" ? "Sent by you" : m.kind === "reminder" ? "Reminder" : m.kind === "auto_reply" ? "Automatic reply" : null;
  switch (m.status) {
    case "queued":
    case "leased":
      return { caption: "Sending…", captionTone: "muted" };
    case "dry_run":
      return { caption: "Dry run: not sent", captionTone: "warn" };
    case "failed":
      return { caption: `Not delivered${m.detail ? `: ${m.detail}` : ""}`, captionTone: "error" };
    default:
      if (isLastOutbound) return { caption: who ? `${who} · Delivered` : "Delivered", captionTone: "muted" };
      return { caption: who, captionTone: "muted" };
  }
}

export function buildThread(messages: ThreadMessage[]): ThreadItem[] {
  const sorted = [...messages].sort((a, b) => (a.createdAt < b.createdAt ? -1 : a.createdAt > b.createdAt ? 1 : 0));
  let lastOutboundIndex = -1;
  sorted.forEach((m, i) => {
    if (m.direction === "out") lastOutboundIndex = i;
  });
  return sorted.map((m, i) => {
    const prev = sorted[i - 1];
    const next = sorted[i + 1];
    const gap = prev ? new Date(m.createdAt).getTime() - new Date(prev.createdAt).getTime() : Infinity;
    const { caption, captionTone } = statusCaption(m, i === lastOutboundIndex);
    return {
      message: m,
      separator: gap > GAP_FOR_SEPARATOR_MS ? separatorLabel(m.createdAt) : null,
      tail:
        !next ||
        next.direction !== m.direction ||
        new Date(next.createdAt).getTime() - new Date(m.createdAt).getTime() > GAP_FOR_SEPARATOR_MS ||
        caption !== null,
      caption,
      captionTone,
    };
  });
}

/**
 * The refusal in a tool result, if any: an is_error result, or a JSON body
 * with an `error` field. Returns null when the tool succeeded.
 */
export function toolRefusal(content: unknown, isError: boolean): string | null {
  const text = typeof content === "string" ? content : JSON.stringify(content ?? "");
  if (isError) return text.slice(0, 160);
  try {
    const parsed = JSON.parse(text) as { error?: unknown };
    if (parsed && typeof parsed === "object" && typeof parsed.error === "string") return parsed.error;
  } catch {
    // Plain-text results are successes.
  }
  return null;
}

/** Human summary of an agent tool call for the activity list. */
export function describeToolCall(name: string, input: Record<string, unknown>): string {
  switch (name) {
    case "remember_note":
      return `Saved a note: ${String(input.text ?? "")}`;
    case "recall_notes":
      return "Read saved notes";
    case "forget_note":
      return "Deleted a note";
    case "set_reminder": {
      const when = typeof input.when === "string" ? new Date(input.when) : null;
      const at = when && !Number.isNaN(when.getTime())
        ? when.toLocaleString("en-US", { weekday: "short", hour: "numeric", minute: "2-digit" })
        : "later";
      return `Scheduled a reminder for ${at}`;
    }
    case "list_reminders":
      return "Checked upcoming reminders";
    case "cancel_reminder":
      return "Cancelled a reminder";
    default:
      return `Used ${name}`;
  }
}
