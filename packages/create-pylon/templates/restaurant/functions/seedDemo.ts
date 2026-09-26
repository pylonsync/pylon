import { mutation } from "@pylonsync/functions";
import { demoDataEnabled, demoReservations } from "../lib/demo";
import { siteConfig } from "../lib/site.config";

// seedDemo — fill an empty reservation book with fictional tables in
// development, so the public calendar and the owner dashboard show the same
// reservations.
//
// The reservation widget calls this when it finds no reserved tables, and the
// dashboard calls it once on load. It writes nothing unless demo data is on
// (lib/demo.ts: PYLON_DEMO_DATA, else a `pylon dev` process), so `pylon start`,
// Docker, and Pylon Cloud deploys do not get demo rows. It also writes nothing
// once any Reservation exists, so it cannot mix demo rows into a real book. The
// advisory lock stops two first visits from seeding twice.
//
// Each active reservation gets its ReservationSlot marker, the same pair
// createReservation writes. Public because the first visitor in `pylon dev` is
// anonymous. It returns only a flag, never a row.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "public",
  async handler(ctx) {
    if (!demoDataEnabled(ctx.env)) return { seeded: false };

    await ctx.db.advisoryLock("restaurant_seed_demo");
    const existing = await ctx.db.unsafe.list("Reservation");
    if (existing.length > 0) return { seeded: false };

    for (const r of demoReservations(Date.now(), siteConfig)) {
      const reservationId = await ctx.db.unsafe.insert("Reservation", {
        startsAt: r.startsAt,
        partySize: r.partySize,
        customerName: r.customerName,
        customerEmail: r.customerEmail,
        customerPhone: r.customerPhone,
        notes: r.notes,
        status: r.status,
        createdAt: r.createdAt,
      });
      if (r.status === "cancelled") continue;
      await ctx.db.unsafe.insert("ReservationSlot", { startsAt: r.startsAt, reservationId });
    }
    return { seeded: true };
  },
});
