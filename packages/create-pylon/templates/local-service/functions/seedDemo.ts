import { mutation } from "@pylonsync/functions";
import { demoBookings, demoDataEnabled } from "../lib/demo";
import { siteConfig } from "../lib/site.config";

// seedDemo — fill an empty booking book with fictional appointments in
// development, so the public grid and the owner dashboard show the same
// bookings.
//
// The booking widget calls this when it finds no booked slots, and the
// dashboard calls it once on load. It writes nothing unless demo data is on
// (lib/demo.ts: PYLON_DEMO_DATA, else a `pylon dev` process), so `pylon start`,
// Docker, and Pylon Cloud deploys do not get demo rows. It also writes nothing
// once any Booking exists, so it cannot mix demo rows into a real book. The
// advisory lock stops two first visits from seeding twice.
//
// Each active booking gets its BookedSlot, the same pair createBooking writes.
// Public because the first visitor in `pylon dev` is anonymous. It returns only
// a flag, never a row.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "public",
  async handler(ctx) {
    if (!demoDataEnabled(ctx.env)) return { seeded: false };

    await ctx.db.advisoryLock("local_service_seed_demo");
    const existing = await ctx.db.unsafe.list("Booking");
    if (existing.length > 0) return { seeded: false };

    for (const b of demoBookings(Date.now(), siteConfig)) {
      const bookingId = await ctx.db.unsafe.insert("Booking", {
        serviceSlug: b.serviceSlug,
        startsAt: b.startsAt,
        endsAt: b.endsAt,
        customerName: b.customerName,
        customerEmail: b.customerEmail,
        customerPhone: b.customerPhone,
        status: b.status,
        createdAt: b.createdAt,
      });
      if (b.status === "cancelled") continue;
      await ctx.db.unsafe.insert("BookedSlot", {
        serviceSlug: b.serviceSlug,
        startsAt: b.startsAt,
        endsAt: b.endsAt,
        bookingId,
      });
    }
    return { seeded: true };
  },
});
