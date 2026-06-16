import { mutation } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";

// seedCapacity — create the single Capacity row from config on first visit
// (idempotent). The landing page calls this on mount; once a row exists it's a
// no-op, so it's safe to call on every load. A lock keeps two concurrent
// first-visits from creating two rows.
//
// Public so an anonymous first visitor seeds it — it only ever writes the
// config's booking window + open-slot count, never reads or returns anything
// sensitive.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("agency_seed_capacity");
    const existing = await ctx.db.unsafe.list("Capacity");
    if (existing.length > 0) return { seeded: false };

    await ctx.db.unsafe.insert("Capacity", {
      label: siteConfig.capacity.label,
      openSlots: siteConfig.capacity.openSlots,
      updatedAt: new Date().toISOString(),
    });
    return { seeded: true };
  },
});
