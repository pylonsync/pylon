import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { primId: v.id("Prim") },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.delete("Prim", args.primId);
    return { ok: true };
  },
});
