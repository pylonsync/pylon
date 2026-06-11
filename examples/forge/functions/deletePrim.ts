import { mutation, v } from "@pylonsync/functions";

export default mutation({
  auth: "guest",
  args: { primId: v.id("Prim") },
  async handler(ctx, args) {
    const a = args as { primId: string };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.delete("Prim", a.primId);
    return { ok: true };
  },
});
