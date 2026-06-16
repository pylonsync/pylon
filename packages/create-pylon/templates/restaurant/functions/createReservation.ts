import { mutation, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

// createReservation — the ONLY writer of Reservation + its ReservationSlot
// marker. A transactional `mutation` (atomic + `ctx.db`); the marker insert
// fires a change event so every open picker recomputes "tables left" live.
//
// `auth: "public"` — a guest has no account. The server RE-CHECKS that the
// seating is still under capacity before inserting, under a per-seating advisory
// lock so two parties can't both grab the last table. Capacity is per seating
// time: count the markers sharing this `startsAt`, compare to tablesPerSlot.
//
// PRIVACY: returns only `{ ok, reason? }` — never a reservation row or any
// guest's contact details.
export default mutation<
  {
    startsAt: string;
    partySize: number;
    customerName: string;
    customerEmail: string;
    customerPhone?: string;
    notes?: string;
  },
  { ok: boolean; reason?: "past" | "full" | "party" | "invalid" }
>({
  auth: "public",
  args: {
    startsAt: v.string(),
    partySize: v.int(),
    customerName: v.string(),
    customerEmail: v.string(),
    customerPhone: v.optional(v.string()),
    notes: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const cfg = siteConfig.reservations;
    const name = args.customerName.trim();
    const email = args.customerEmail.trim().toLowerCase();
    const phone = args.customerPhone?.trim() || null;
    const notes = args.notes?.trim() || null;

    if (name.length < 1 || name.length > 120) {
      throw ctx.error("INVALID_ARGS", "Enter your name.");
    }
    if (!EMAIL_RE.test(email) || email.length > 254) {
      throw ctx.error("INVALID_ARGS", "Enter a valid email address.");
    }
    if (!Number.isInteger(args.partySize) || args.partySize < 1) {
      return { ok: false, reason: "party" };
    }
    if (args.partySize > cfg.maxPartySize) {
      return { ok: false, reason: "party" };
    }

    const startMs = Date.parse(args.startsAt);
    if (Number.isNaN(startMs)) return { ok: false, reason: "invalid" };
    const startsAt = new Date(startMs).toISOString();
    if (startMs < Date.now() + cfg.leadTimeHours * 3_600_000) {
      return { ok: false, reason: "past" };
    }

    // Serialize the count-then-insert for THIS seating so two parties can't
    // both claim the last table. Held until the tx commits.
    await ctx.db.advisoryLock(`reservation_slot:${startsAt}`);

    // Capacity check: how many tables are already taken at this seating?
    // Cross-user read of the deny-all-projection → `unsafe`.
    const markers = (await ctx.db.unsafe.list("ReservationSlot")) as unknown as {
      startsAt: string;
    }[];
    const taken = markers.filter((m) => m.startsAt === startsAt).length;
    if (taken >= cfg.tablesPerSlot) {
      return { ok: false, reason: "full" };
    }

    const reservationId = await ctx.db.unsafe.insert("Reservation", {
      startsAt,
      partySize: args.partySize,
      customerName: name,
      customerEmail: email,
      customerPhone: phone,
      notes,
      status: "pending",
      createdAt: new Date().toISOString(),
    });
    await ctx.db.unsafe.insert("ReservationSlot", { startsAt, reservationId });

    return { ok: true };
  },
});
