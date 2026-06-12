import { mutation } from "@pylonsync/functions";

/**
 * Scheduled sweep: restore every destroyed building, then reschedule
 * itself — a self-perpetuating chain (the framework pattern for
 * recurring work; scheduled jobs persist to SQLite, so the chain
 * survives server restarts). spawnAvatar bootstraps the chain and
 * revives it if the heartbeat goes stale.
 *
 * Keep SWEEP_MS in sync with the copy of this constant in
 * spawnAvatar.ts (functions/ files are each a standalone endpoint, so
 * they can't share a module).
 */
const SWEEP_MS = 10 * 60 * 1000;
const MARKER_KEY = "buildingSweep";

export default mutation({
  // Scheduler-driven (runs with the chain-starter's guest identity),
  // but also directly callable — it's no more powerful than the
  // public resetIsland button.
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    // Serialize against concurrent sweeps and spawnAvatar's
    // chain-revival check.
    await ctx.db.advisoryLock("world3d.buildingSweep");

    // The Destruction policy lets any signed-in player delete rows
    // (same as resetIsland), so the plain surface is correct here.
    const rows = await ctx.db.query("Destruction", {});
    for (const row of rows) await ctx.db.delete("Destruction", row.id as string);

    // Chain dedupe: if the heartbeat already points well past now,
    // another chain claimed the next slot (clock jump, late job +
    // spawn revival racing) — let it own the cadence and end this one.
    const marker = await ctx.db.lookup("World", "key", MARKER_KEY);
    const now = Date.now();
    if (marker && new Date(marker.nextRunAt as string).getTime() > now + 60_000) {
      return { deleted: rows.length, rescheduled: false };
    }

    const nextRunAt = new Date(now + SWEEP_MS).toISOString();
    // unsafe: World is policy-locked to server functions; the
    // heartbeat row is framework-internal state, not player data.
    if (marker) {
      await ctx.db.unsafe.update("World", marker.id as string, { nextRunAt });
    } else {
      await ctx.db.unsafe.insert("World", { key: MARKER_KEY, nextRunAt });
    }
    await ctx.scheduler.runAfter(SWEEP_MS, "rebuildBuildings", {});
    return { deleted: rows.length, rescheduled: true };
  },
});
