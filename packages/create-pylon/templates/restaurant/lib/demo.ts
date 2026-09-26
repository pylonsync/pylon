// Demo reservations for local development.
//
// `pylon dev` fills an empty reservation book with fictional tables from the
// past week and the next two weeks, so the calendar shows fewer tables left
// (and some full seatings) and the owner dashboard has a book to run. Both
// read the same rows: every active demo Reservation gets its ReservationSlot
// marker, exactly like createReservation writes.
//
// A deploy does not seed: `demoDataEnabled` is true only when PYLON_DEMO_DATA is
// on, or when it is unset and the process runs under `pylon dev`. Set PYLON_DEMO_DATA=0 to
// turn it off in development too.
//
// Pure data + pure shaping, so functions/seedDemo.ts stays a thin wrapper and
// this file is testable without a server.

import { localDateKey, seatingsForDay, weekdayOf } from "./slots";
import type { RestaurantConfig } from "./site.config";

const ON = new Set(["1", "true", "yes", "on"]);
const OFF = new Set(["0", "false", "no", "off"]);

/**
 * True when demo rows may be written. An explicit PYLON_DEMO_DATA wins.
 * Otherwise it needs both PYLON_DEV_MODE on and PYLON_DEV_WATCH_DIR set. Only
 * `pylon dev` sets PYLON_DEV_WATCH_DIR; `pylon start` and the Docker image do
 * not, and the image turns PYLON_DEV_MODE on by default, so dev mode alone is
 * not proof of a local machine.
 */
export function demoDataEnabled(env: Record<string, string | undefined>): boolean {
  const explicit = env.PYLON_DEMO_DATA?.trim().toLowerCase();
  if (explicit && ON.has(explicit)) return true;
  if (explicit && OFF.has(explicit)) return false;
  const dev = env.PYLON_DEV_MODE?.trim().toLowerCase();
  const devMode = dev === "1" || dev === "true";
  return devMode && Boolean(env.PYLON_DEV_WATCH_DIR?.trim());
}

const GUESTS = [
  "Ana Whitaker", "James Osei", "Priya Raman", "Tom Alvarez", "Claire Dubois", "Marcus Iyer",
  "Leah Goldberg", "Noah Fischer", "Sofia Marin", "Daniel Park", "Grace Mensah", "Henry Walsh",
  "Isabel Cruz", "Owen Reilly", "Nadia Haddad", "Ethan Brooks", "Maya Tanaka", "Luca Romano",
  "Zoe Bennett", "Samir Khan", "Ruth Castillo", "Felix Wagner", "Hana Sato", "Diego Vega",
];
const NOTES = [
  "Anniversary dinner",
  "One guest is vegetarian",
  "Window table if possible",
  "Celebrating a birthday",
  "Shellfish allergy",
  "Arriving from the theater, may run 10 minutes late",
];

/** Small deterministic PRNG (mulberry32) so the demo set is stable per seed. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export interface DemoReservation {
  startsAt: string;
  partySize: number;
  customerName: string;
  customerEmail: string;
  customerPhone: string | null;
  notes: string | null;
  status: "pending" | "confirmed" | "cancelled";
  createdAt: string;
}

type Cfg = Pick<RestaurantConfig, "reservations">;

/**
 * Reservations on open days from 7 days ago to 13 days ahead, on the seating
 * grid. Active reservations never exceed `tablesPerSlot` at one seating.
 * Weekend evenings around 7–8 PM fill up; early and late seatings stay light.
 */
export function demoReservations(nowMs: number, cfg: Cfg, seed = 5): DemoReservation[] {
  const r = cfg.reservations;
  const rand = rng(seed);
  const out: DemoReservation[] = [];

  for (let offset = -7; offset <= 13; offset++) {
    const day = localDateKey(offset, nowMs);
    const dow = weekdayOf(day);
    const hours = r.hours[dow];
    if (!hours) continue;
    const seatings = seatingsForDay({
      dayISODate: day,
      open: hours.open,
      close: hours.close,
      slotMinutes: r.slotMinutes,
      leadTimeHours: 0,
      nowMs: 0,
    });
    const weekend = dow === 5 || dow === 6;
    // Nearer dates are more booked than dates two weeks out.
    const horizon = offset <= 3 ? 1 : offset <= 8 ? 0.7 : 0.4;
    for (const seat of seatings) {
      const hour = new Date(seat.startsAt).getHours();
      const peak = hour >= 19 && hour < 21 ? 1 : hour >= 18 && hour < 22 ? 0.6 : 0.3;
      const expected = r.tablesPerSlot * peak * horizon * (weekend ? 0.95 : 0.5);
      let tables = Math.min(r.tablesPerSlot, Math.round(expected + (rand() - 0.4) * 2));
      if (tables < 0) tables = 0;
      const start = Date.parse(seat.startsAt);
      let active = 0;
      for (let t = 0; t < tables + 1 && active < r.tablesPerSlot; t++) {
        // The extra pass may add a cancelled row, which frees its table.
        const extra = t === tables;
        if (extra && rand() > 0.15) break;
        const past = start <= nowMs;
        const roll = rand();
        const status: DemoReservation["status"] = extra
          ? "cancelled"
          : past
            ? "confirmed"
            : roll < 0.86
              ? "confirmed"
              : "pending";
        if (status !== "cancelled") active++;
        const name = GUESTS[Math.floor(rand() * GUESTS.length)];
        const [first, last] = name.toLowerCase().split(" ");
        const party = rand() < 0.55 ? 2 : Math.min(r.maxPartySize, 3 + Math.floor(rand() * 4));
        const bookedAt = Math.min(nowMs - 3_600_000, start - (1 + Math.floor(rand() * 10)) * 86_400_000);
        out.push({
          startsAt: seat.startsAt,
          partySize: party,
          customerName: name,
          customerEmail: `${first}.${last}@example.com`,
          customerPhone: rand() < 0.75 ? `(214) 555-01${String(Math.floor(rand() * 100)).padStart(2, "0")}` : null,
          notes: rand() < 0.18 ? NOTES[Math.floor(rand() * NOTES.length)] : null,
          status,
          createdAt: new Date(bookedAt).toISOString(),
        });
      }
    }
  }
  return out;
}
