import { mutation } from "@pylonsync/functions";
import { seedTiles } from "./lib/seed";

/**
 * Idempotent first-boot demo city so the lobby's "Load a city" list is never
 * an empty shell. The client calls this on the lobby; the first caller creates
 * a fixed-key "Pylon City" with a starter neighbourhood and starts its sim,
 * and every later caller just keeps that city's tick chain alive. The advisory
 * lock + fixed key make concurrent visitors safe (no duplicate demo cities).
 */
const TICK_MS = 6000;
const START_FUNDS = 8000;
const DEMO_KEY = "pylon-city";
const DEMO_NAME = "Pylon City";

export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.advisoryLock("sim.seedDemo");

    const now = Date.now();
    const existing = await ctx.db.lookup("City", "key", DEMO_KEY);
    if (existing) {
      // Keep the demo city's tick alive (revive after a restart).
      const due = new Date(existing.nextTickAt as string).getTime();
      if (!Number.isFinite(due) || due < now - 60_000) {
        await ctx.db.unsafe.update("City", existing.id as string, {
          nextTickAt: new Date(now + TICK_MS).toISOString(),
        });
        await ctx.scheduler.runAfter(TICK_MS, "cityTick", { cityId: DEMO_KEY });
      }
      return { cityId: DEMO_KEY, created: false };
    }

    const nowIso = new Date(now).toISOString();
    // unsafe: City is policy-locked to server functions.
    await ctx.db.unsafe.insert("City", {
      key: DEMO_KEY,
      name: DEMO_NAME,
      funds: START_FUNDS,
      population: 0,
      jobs: 0,
      happiness: 100,
      tick: 0,
      nextTickAt: new Date(now + TICK_MS).toISOString(),
      updatedAt: nowIso,
    });
    for (const t of seedTiles()) {
      await ctx.db.unsafe.insert("Tile", {
        cityId: DEMO_KEY,
        gx: t.gx,
        gz: t.gz,
        kind: t.kind,
        level: t.level,
        userId: "seed",
        updatedAt: nowIso,
      });
    }
    await ctx.scheduler.runAfter(TICK_MS, "cityTick", { cityId: DEMO_KEY });
    return { cityId: DEMO_KEY, created: true };
  },
});
