import { mutation } from "@pylonsync/functions";

/**
 * Sweep all bot dots. Used by the "Clear bots" button in the UI —
 * useful when you've stress-tested with 1000+ bots and want a reset.
 */
export default mutation({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const bots = await ctx.db.query("Dot", { isBot: true });
    for (const b of bots) await ctx.db.delete("Dot", b.id as string);
    return { deleted: bots.length };
  },
});
