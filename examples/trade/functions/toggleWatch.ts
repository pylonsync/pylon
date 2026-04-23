import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    userId: v.string(),
    symbol: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const existing = await ctx.db.query("Watch", {
      userId: args.userId, symbol: args.symbol,
    });
    if (existing.length > 0) {
      await ctx.db.delete("Watch", existing[0].id as string);
      return { watching: false };
    }
    await ctx.db.insert("Watch", {
      userId: args.userId,
      symbol: args.symbol,
      addedAt: new Date().toISOString(),
    });
    return { watching: true };
  },
});
