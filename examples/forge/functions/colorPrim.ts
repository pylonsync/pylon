import { mutation, v } from "@pylonsync/functions";

export default mutation({
  auth: "guest",
  args: {
    primId: v.id("Prim"),
    color: v.string(),
  },
  async handler(ctx, args) {
    const a = args as { primId: string; color: string };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.update("Prim", a.primId, {
      color: a.color,
      updatedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
