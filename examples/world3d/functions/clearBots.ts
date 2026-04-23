import { mutation } from "@pylonsync/functions";

export default mutation({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const bots = await ctx.db.query("Avatar", { isBot: true });
    for (const b of bots) await ctx.db.delete("Avatar", b.id as string);
    return { deleted: bots.length };
  },
});
