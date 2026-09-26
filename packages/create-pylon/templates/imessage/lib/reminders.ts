// Bounds for reminders the assistant schedules. The model picks the time; the
// server decides whether it is acceptable.

export const MIN_REMINDER_LEAD_MS = 5 * 60_000;
export const MAX_REMINDER_HORIZON_MS = 366 * 24 * 60 * 60 * 1000;
export const MAX_OPEN_REMINDERS_PER_CONTACT = 20;
/** Reminders the assistant may create for one contact in a rolling day, sent or not. */
export const MAX_REMINDERS_PER_CONTACT_PER_DAY = 10;
export const MAX_REMINDER_CHARS = 500;
export const MAX_NOTE_CHARS = 500;
export const MAX_NOTES_PER_CONTACT = 100;

export type ReminderTimeCheck = { ok: true; dueAt: string; dueMs: number } | { ok: false; error: string };

/**
 * Validate a reminder time. Requires an explicit offset or `Z` so "9am" is
 * never silently read in the server's timezone.
 */
export function checkReminderTime(when: string, now: Date): ReminderTimeCheck {
  const trimmed = when.trim();
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})$/.test(trimmed)) {
    return {
      ok: false,
      error: "Use an ISO 8601 time with a UTC offset, for example 2026-10-03T09:00:00-05:00.",
    };
  }
  const ms = Date.parse(trimmed);
  if (!Number.isFinite(ms)) return { ok: false, error: "That time could not be parsed." };
  if (ms < now.getTime() + MIN_REMINDER_LEAD_MS) {
    return { ok: false, error: "The reminder time must be at least five minutes in the future." };
  }
  if (ms > now.getTime() + MAX_REMINDER_HORIZON_MS) {
    return { ok: false, error: "Reminders can be at most one year out." };
  }
  return { ok: true, dueAt: new Date(ms).toISOString(), dueMs: ms };
}

/** Collapse whitespace and cap length for text the model wants to store. */
export function cleanStoredText(text: string, max: number): string {
  return text.replace(/\s+/g, " ").trim().slice(0, max);
}
