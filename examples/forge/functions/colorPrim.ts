import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    primId: v.id("Prim"),
    color: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.update("Prim", args.primId, {
      color: args.color,
      updatedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
