import { mutation, v } from "@pylonsync/functions";

export default mutation({
  auth: "guest",
  args: {
    primId: v.id("Prim"),
    x: v.number(),
    y: v.number(),
    z: v.number(),
  },
  async handler(ctx, args) {
    const a = args as { primId: string; x: number; y: number; z: number };
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const p = await ctx.db.get("Prim", a.primId);
    if (!p) throw ctx.error("NOT_FOUND", "primitive not found");
    await ctx.db.update("Prim", a.primId, {
      x: a.x, y: a.y, z: a.z,
      updatedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
