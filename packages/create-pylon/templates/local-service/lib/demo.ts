// Demo bookings for local development.
//
// `pylon dev` fills an empty booking book with fictional appointments from the
// past week and the next ten days, so the booking grid shows taken times and
// the owner dashboard has a schedule. Both read the same rows: every demo
// Booking gets its BookedSlot, exactly like createBooking writes.
//
// A deploy does not seed: `demoDataEnabled` is true only when PYLON_DEMO_DATA is
// on, or when it is unset and the process runs under `pylon dev`. Set PYLON_DEMO_DATA=0 to
// turn it off in development too.
//
// Pure data + pure shaping, so functions/seedDemo.ts stays a thin wrapper and
// this file is testable without a server.

import { localDateKey, rangesOverlap, slotsForDay, weekdayOf } from "./slots";
import type { LocalServiceConfig } from "./site.config";

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

const CUSTOMERS = [
  "Marcus Bell", "Daniel Reyes", "Hannah Kim", "Luis Ortega", "Tyler Brooks", "Andre Wallace",
  "Sam Okafor", "Kevin Tran", "Jordan Miles", "Eli Navarro", "Chris Delgado", "Owen Price",
  "Ravi Shah", "Nate Coleman", "Isaac Moreno", "Ben Hollis", "Victor Lam", "Grant Ellison",
  "Mateo Vargas", "Dev Patel", "Caleb Ford", "Jamal Hayes", "Theo Lindqvist", "Omar Haddad",
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

export interface DemoBooking {
  serviceSlug: string;
  startsAt: string;
  endsAt: string;
  customerName: string;
  customerEmail: string;
  customerPhone: string | null;
  status: "pending" | "confirmed" | "cancelled";
  createdAt: string;
}

type Cfg = Pick<LocalServiceConfig, "booking" | "services">;

/**
 * Appointments on open days from 7 days ago to 9 days ahead. Each fits inside
 * opening hours on the slot grid, and no two active bookings overlap. Past
 * appointments are confirmed; upcoming ones are mostly confirmed, some
 * pending, a few cancelled.
 */
export function demoBookings(nowMs: number, cfg: Cfg, seed = 11): DemoBooking[] {
  const rand = rng(seed);
  const out: DemoBooking[] = [];
  const services = cfg.services.items;
  if (services.length === 0) return out;

  for (let offset = -7; offset <= 9; offset++) {
    const day = localDateKey(offset, nowMs);
    const hours = cfg.booking.hours[weekdayOf(day)];
    if (!hours) continue;
    // The full grid for the day, ignoring "now": seeding also fills the past.
    const grid = slotsForDay({
      dayISODate: day,
      open: hours.open,
      close: hours.close,
      slotMinutes: cfg.booking.slotMinutes,
      durationMin: 0,
      leadTimeHours: 0,
      busy: [],
      nowMs: 0,
    });
    // With a zero duration the last grid start is closing time itself.
    const close = Date.parse(grid[grid.length - 1].startsAt);
    const busy: [number, number][] = [];
    // Busier near the weekend and in the days just ahead.
    const fill = offset >= 0 && offset <= 3 ? 0.5 : 0.35;
    for (const slot of grid) {
      if (rand() > fill) continue;
      const svc = services[Math.floor(rand() * services.length)];
      const start = Date.parse(slot.startsAt);
      const end = start + svc.durationMin * 60_000;
      if (end > close) continue;
      if (busy.some(([s, e]) => rangesOverlap(start, end, s, e))) continue;

      const name = CUSTOMERS[Math.floor(rand() * CUSTOMERS.length)];
      const [first, last] = name.toLowerCase().split(" ");
      const past = end <= nowMs;
      const roll = rand();
      const status: DemoBooking["status"] = past
        ? "confirmed"
        : roll < 0.62
          ? "confirmed"
          : roll < 0.92
            ? "pending"
            : "cancelled";
      if (status !== "cancelled") busy.push([start, end]);
      const bookedAt = Math.min(nowMs - 3_600_000, start - (1 + Math.floor(rand() * 6)) * 86_400_000);
      out.push({
        serviceSlug: svc.slug,
        startsAt: new Date(start).toISOString(),
        endsAt: new Date(end).toISOString(),
        customerName: name,
        customerEmail: `${first}.${last[0]}@example.com`,
        customerPhone: rand() < 0.7 ? `(214) 555-01${String(Math.floor(rand() * 100)).padStart(2, "0")}` : null,
        status,
        createdAt: new Date(bookedAt).toISOString(),
      });
    }
  }
  return out;
}
