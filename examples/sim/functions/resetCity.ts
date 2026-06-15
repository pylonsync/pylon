import { mutation, v } from "@pylonsync/functions";

/**
 * Clear one city back to an empty buildable map and refill its treasury. Any
 * signed-in player can reset — it's a demo sandbox. Bulldozes every tile in
 * the city (the map starts blank, same as a freshly created city) rather than
 * stamping down a generated town.
 */
const START_FUNDS = 8000;

export default mutation({
  auth: "guest",
  args: { cityId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const cityId = String(args.cityId ?? "");
    if (!cityId) throw ctx.error("INVALID_ARGS", "cityId required");

    const existing = await ctx.db.query("Tile", { cityId });
    let deleted = 0;
    for (const t of existing) {
      await ctx.db.delete("Tile", t.id as string);
      deleted++;
    }

    const nowIso = new Date().toISOString();
    const city = await ctx.db.lookup("City", "key", cityId);
    if (city) {
      // unsafe: City is server-owned.
      await ctx.db.unsafe.update("City", city.id as string, {
        funds: START_FUNDS,
        population: 0,
        jobs: 0,
        happiness: 100,
        updatedAt: nowIso,
      });
    }
    return { reset: true, deleted };
  },
});
