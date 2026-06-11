import { mutation, v } from "@pylonsync/functions";

export default mutation({
  // Public demo: anyone with a guest session (POST /api/auth/guest) can manage
  // their watchlist. Without this the function defaults to auth: "user" and
  // rejects guests.
  auth: "guest",
  args: {
    symbol: v.string(),
  },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const existing = await ctx.db.query("Watch", {
      userId, symbol: args.symbol,
    });
    if (existing.length > 0) {
      await ctx.db.delete("Watch", existing[0].id as string);
      return { watching: false };
    }
    await ctx.db.insert("Watch", {
      userId,
      symbol: args.symbol,
      addedAt: new Date().toISOString(),
    });
    return { watching: true };
  },
});
