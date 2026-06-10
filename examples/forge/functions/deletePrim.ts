import { mutation, v } from "@pylonsync/functions";

export default mutation({
  // Public demo: anyone with a guest session (POST /api/auth/guest) can call.
  // Without this the function defaults to auth: "user" and rejects guests.
  auth: "guest",
  args: { primId: v.id("Prim") },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    await ctx.db.delete("Prim", args.primId);
    return { ok: true };
  },
});
