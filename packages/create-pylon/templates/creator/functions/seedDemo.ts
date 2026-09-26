import { mutation } from "@pylonsync/functions";
import { demoDataEnabled, demoSubscribers } from "../lib/demo";
import { syncSubscriberCount } from "../lib/stats";

// seedDemo — fill an empty subscriber list with fictional rows in development.
//
// The landing page counter calls this when it finds no subscribers, and the
// dashboard calls it once on load. It writes nothing unless demo data is on
// (lib/demo.ts: PYLON_DEMO_DATA, else a `pylon dev` process), so `pylon start`,
// Docker, and Pylon Cloud deploys do not get demo rows. It also writes nothing
// once any Subscriber exists, so it cannot mix demo rows into a real list.
// Clearing every subscriber in development seeds the list again on the next
// load. The advisory lock stops two first visits from seeding twice.
//
// Public because the first visitor in `pylon dev` is anonymous. It returns only
// a flag, never a row.
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "public",
  async handler(ctx) {
    if (!demoDataEnabled(ctx.env)) return { seeded: false };

    await ctx.db.advisoryLock("creator_seed_demo");
    const existing = await ctx.db.unsafe.list("Subscriber");
    if (existing.length > 0) return { seeded: false };

    for (const row of demoSubscribers(Date.now())) {
      await ctx.db.unsafe.insert("Subscriber", { email: row.email, createdAt: row.createdAt });
    }
    await syncSubscriberCount(ctx.db.unsafe);
    return { seeded: true };
  },
});
