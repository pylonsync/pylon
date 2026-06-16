// Pure slot math — shared by the client time-picker (which slots to show, which
// to grey out) and the server-side createBooking re-check (does this slot still
// fit). Keeping it dependency-free and in one place means the two sides agree
// exactly: the client greys a taken slot, and the server independently refuses
// to double-book it.
//
// Times are absolute instants (epoch ms / ISO-8601 UTC). Slot GENERATION uses
// local wall-clock hours (the business's open/close, interpreted in the
// runtime's local timezone); overlap + past checks compare absolute instants,
// so they're timezone-agnostic and can't be fooled by string formatting.

export interface BusyRange {
  startsAt: string;
  endsAt: string;
}

export interface Slot {
  /** ISO-8601 start instant. */
  startsAt: string;
  /** ISO-8601 end instant (start + service duration). */
  endsAt: string;
  /** False when the slot is in the past (within lead time) or overlaps a booking. */
  available: boolean;
}

/** Half-open interval overlap: [aStart,aEnd) intersects [bStart,bEnd). */
export function rangesOverlap(
  aStart: number,
  aEnd: number,
  bStart: number,
  bEnd: number,
): boolean {
  return aStart < bEnd && bStart < aEnd;
}

/**
 * Generate the bookable slots for one calendar day for one service.
 *
 * `dayISODate` is a local "YYYY-MM-DD"; `open`/`close` are local "HH:MM". Each
 * slot starts every `slotMinutes` and lasts `durationMin`; the last slot must
 * end by close. A slot is unavailable if it starts before `now + leadTimeHours`
 * or overlaps any `busy` range.
 */
export function slotsForDay(opts: {
  dayISODate: string;
  open: string;
  close: string;
  slotMinutes: number;
  durationMin: number;
  leadTimeHours: number;
  busy: BusyRange[];
  nowMs: number;
}): Slot[] {
  const { dayISODate, open, close, slotMinutes, durationMin, leadTimeHours, busy, nowMs } = opts;
  const [y, m, d] = dayISODate.split("-").map(Number);
  const [oh, om] = open.split(":").map(Number);
  const [ch, cm] = close.split(":").map(Number);
  if ([y, m, d, oh, om, ch, cm].some((n) => Number.isNaN(n))) return [];

  // `new Date(y, m-1, d, h, min)` builds a LOCAL time; `.getTime()` is the
  // correct absolute instant regardless of timezone.
  const dayStart = new Date(y, m - 1, d, oh, om, 0, 0).getTime();
  const dayEnd = new Date(y, m - 1, d, ch, cm, 0, 0).getTime();
  const minStart = nowMs + leadTimeHours * 3_600_000;
  const step = slotMinutes * 60_000;
  const dur = durationMin * 60_000;
  const busyMs = busy.map((b) => [Date.parse(b.startsAt), Date.parse(b.endsAt)] as const);

  const slots: Slot[] = [];
  for (let s = dayStart; s + dur <= dayEnd; s += step) {
    const e = s + dur;
    const past = s < minStart;
    const taken = busyMs.some(([bs, be]) => rangesOverlap(s, e, bs, be));
    slots.push({
      startsAt: new Date(s).toISOString(),
      endsAt: new Date(e).toISOString(),
      available: !past && !taken,
    });
  }
  return slots;
}

/** The weekday (0=Sun..6=Sat) for a local "YYYY-MM-DD". */
export function weekdayOf(dayISODate: string): number {
  const [y, m, d] = dayISODate.split("-").map(Number);
  return new Date(y, m - 1, d).getDay();
}

/** Local "YYYY-MM-DD" for an instant `daysFromToday` away (0 = today). */
export function localDateKey(daysFromToday: number, nowMs: number): string {
  const d = new Date(nowMs);
  d.setDate(d.getDate() + daysFromToday);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}
