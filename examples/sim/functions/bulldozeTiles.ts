import { mutation, v } from "@pylonsync/functions";

/**
 * Clear tiles from a city's map and refund a fraction of their cost.
 * Refund lands in that city's treasury (City row, written via unsafe).
 *
 * Costs mirror game/config.ts.
 */
const COST: Record<string, number> = { road: 10, res: 20, com: 30, ind: 25 };
const REFUND = 0.3;

export default mutation({
  auth: "guest",
  args: {
    cityId: v.string(),
    cells: v.array(v.object({ gx: v.int(), gz: v.int() })),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const cityId = String(args.cityId ?? "");
    if (!cityId) throw ctx.error("INVALID_ARGS", "cityId required");
    const cells = args.cells as Array<{ gx: number; gz: number }>;
    if (cells.length === 0) return { cleared: 0, refunded: 0 };
    if (cells.length > 128) throw ctx.error("INVALID_ARGS", "too many cells in one call");

    const city = await ctx.db.lookup("City", "key", cityId);
    let refunded = 0;
    let cleared = 0;
    for (const { gx, gz } of cells) {
      const existing = await ctx.db.query("Tile", { cityId, gx, gz });
      const row = existing[0];
      if (!row) continue;
      await ctx.db.delete("Tile", row.id as string);
      refunded += Math.round((COST[row.kind as string] ?? 0) * REFUND);
      cleared++;
    }

    if (cleared > 0 && city) {
      const now = new Date().toISOString();
      // unsafe: City is server-owned.
      await ctx.db.unsafe.update("City", city.id as string, {
        funds: (city.funds as number) + refunded,
        updatedAt: now,
      });
    }
    return { cleared, refunded };
  },
});
