// Pure seating-time math — shared by the client picker (which times to show,
// how many tables are left) and the server createReservation re-check (is this
// seating still under capacity). Dependency-free + one source of truth so the
// two sides agree exactly.
//
// Reservations sit at fixed seating times (every `slotMinutes` within hours).
// Capacity is per-seating: each seating has `tablesPerSlot` tables, and a
// reservation takes one. So availability is a COUNT (how many markers share a
// startsAt), not an overlap — simpler than duration-based booking.
//
// Times are absolute instants (ISO-8601 UTC); seating GENERATION uses local
// wall-clock hours. Comparing seating instants is timezone-agnostic.

/** ISO start instants for every seating on a local calendar day. */
export function seatingsForDay(opts: {
  dayISODate: string; // local "YYYY-MM-DD"
  open: string; // local "HH:MM"
  close: string; // last seating must be <= close
  slotMinutes: number;
  leadTimeHours: number;
  nowMs: number;
}): { startsAt: string; past: boolean }[] {
  const { dayISODate, open, close, slotMinutes, leadTimeHours, nowMs } = opts;
  const [y, m, d] = dayISODate.split("-").map(Number);
  const [oh, om] = open.split(":").map(Number);
  const [ch, cm] = close.split(":").map(Number);
  if ([y, m, d, oh, om, ch, cm].some((n) => Number.isNaN(n))) return [];

  const start = new Date(y, m - 1, d, oh, om, 0, 0).getTime();
  const end = new Date(y, m - 1, d, ch, cm, 0, 0).getTime();
  const minStart = nowMs + leadTimeHours * 3_600_000;
  const step = slotMinutes * 60_000;

  const out: { startsAt: string; past: boolean }[] = [];
  for (let s = start; s <= end; s += step) {
    out.push({ startsAt: new Date(s).toISOString(), past: s < minStart });
  }
  return out;
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
