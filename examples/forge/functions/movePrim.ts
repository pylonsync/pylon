import { mutation, v } from "@pylonsync/functions";

export default mutation({
  // Public demo: anyone with a guest session (POST /api/auth/guest) can call.
  // Without this the function defaults to auth: "user" and rejects guests.
  auth: "guest",
  args: {
    primId: v.id("Prim"),
    x: v.number(),
    y: v.number(),
    z: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const p = await ctx.db.get("Prim", args.primId);
    if (!p) throw ctx.error("NOT_FOUND", "primitive not found");
    await ctx.db.update("Prim", args.primId, {
      x: args.x, y: args.y, z: args.z,
      updatedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
