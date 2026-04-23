import { mutation, v } from "@pylonsync/functions";

/**
 * Record a single trade tick. Updates the Ticker's price/volume and
 * appends to the Trade log. The market-maker tab calls this in a tight
 * loop to drive the dashboard; real deployments would have the server
 * run a cron/scheduled job instead of a client loop.
 */
export default mutation({
  args: {
    symbol: v.string(),
    price: v.number(),
    qty: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const rows = await ctx.db.query("Ticker", { symbol: args.symbol });
    if (rows.length === 0) throw ctx.error("NOT_FOUND", `ticker ${args.symbol}`);
    const t = rows[0];

    const now = new Date().toISOString();
    await ctx.db.update("Ticker", t.id as string, {
      price: args.price,
      volume: (t.volume as number) + args.qty,
      dayHigh: Math.max(t.dayHigh as number, args.price),
      dayLow: Math.min(t.dayLow as number, args.price),
      updatedAt: now,
    });

    await ctx.db.insert("Trade", {
      symbol: args.symbol,
      price: args.price,
      qty: args.qty,
      at: now,
    });
    return { ok: true };
  },
});
