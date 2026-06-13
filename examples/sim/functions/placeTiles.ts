import { mutation, v } from "@pylonsync/functions";

/**
 * Paint tiles onto the shared map and charge the shared treasury.
 *
 * Server-authoritative on money and occupancy: each cell is bounds-
 * and kind-checked, skipped if already occupied (placement never
 * overwrites — bulldoze first), and charged only if the city can
 * afford it. Funds are written via ctx.db.unsafe because the City row
 * is policy-locked; tile inserts go through the normal surface (the
 * Tile insert policy is "any signed-in player").
 *
 * Costs mirror game/config.ts (functions can't import it).
 */
const GRID = 128;
const COST: Record<string, number> = { road: 10, res: 20, com: 30, ind: 25 };

export default mutation({
  auth: "guest",
  args: {
    cells: v.array(
      v.object({ gx: v.int(), gz: v.int(), kind: v.string() }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const cells = args.cells as Array<{ gx: number; gz: number; kind: string }>;
    if (cells.length === 0) return { placed: 0, spent: 0, funds: 0 };
    if (cells.length > 128) throw ctx.error("INVALID_ARGS", "too many cells in one call");

    const city = await ctx.db.lookup("City", "key", "main");
    if (!city) throw ctx.error("NOT_FOUND", "city not initialised");
    let funds = city.funds as number;

    const now = new Date().toISOString();
    let placed = 0;
    let spent = 0;
    for (const cell of cells) {
      const { gx, gz, kind } = cell;
      if (gx < 0 || gz < 0 || gx >= GRID || gz >= GRID) continue;
      const cost = COST[kind];
      if (cost === undefined) continue; // unknown kind
      if (funds < cost) break; // out of money — stop here
      const existing = await ctx.db.query("Tile", { gx, gz });
      if (existing.length > 0) continue; // occupied; bulldoze first

      await ctx.db.insert("Tile", {
        gx,
        gz,
        kind,
        level: 0,
        userId: ctx.auth.userId,
        updatedAt: now,
      });
      funds -= cost;
      spent += cost;
      placed++;
    }

    if (placed > 0) {
      // unsafe: City is server-owned (see app.ts policy).
      await ctx.db.unsafe.update("City", city.id as string, { funds, updatedAt: now });
    }
    return { placed, spent, funds };
  },
});
