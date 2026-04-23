import { mutation } from "@pylonsync/functions";

export default mutation({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const counters = await ctx.db.query("Counter", {});
    for (const c of counters) await ctx.db.delete("Counter", c.id as string);
    const samples = await ctx.db.query("Sample", {});
    for (const s of samples) await ctx.db.delete("Sample", s.id as string);
    return { deleted: counters.length + samples.length };
  },
});
