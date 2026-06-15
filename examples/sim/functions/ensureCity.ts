import { mutation, v } from "@pylonsync/functions";

/**
 * Keep an existing city's simulation alive. The client calls this when it
 * enters a city (from the lobby) so the self-perpetuating tick chain is
 * revived after a server restart / wiped job store. Under an advisory lock so
 * concurrent joins can't double-start it. Creating a city is createCity's job;
 * this is a no-op if the city doesn't exist.
 */
const TICK_MS = 6000;

export default mutation({
  auth: "guest",
  args: { cityId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const cityId = String(args.cityId ?? "");
    if (!cityId) throw ctx.error("INVALID_ARGS", "cityId required");

    await ctx.db.advisoryLock(`sim.city.${cityId}`);
    const city = await ctx.db.lookup("City", "key", cityId);
    if (!city) return { revived: false, missing: true };

    const now = Date.now();
    // Revive the tick chain if its heartbeat is missing or stale.
    const due = new Date(city.nextTickAt as string).getTime();
    if (!Number.isFinite(due) || due < now - 60_000) {
      const nextTickAt = new Date(now + TICK_MS).toISOString();
      await ctx.db.unsafe.update("City", city.id as string, { nextTickAt });
      await ctx.scheduler.runAfter(TICK_MS, "cityTick", { cityId });
      return { revived: true };
    }
    return { revived: false };
  },
});
