import { mutation, v } from "@pylonsync/functions";

/**
 * Bring your own avatar back to full health after a death. The client
 * teleports itself to a fresh spawn point (it knows the terrain) and
 * calls this; ownership is enforced so nobody can heal someone else.
 */
export default mutation({
  auth: "guest",
  args: {
    avatarId: v.id("Avatar"),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const avatarId = args.avatarId as string;
    const row = await ctx.db.get("Avatar", avatarId);
    if (!row) throw ctx.error("NOT_FOUND", "avatar not found");
    if (row.userId !== ctx.auth.userId) {
      throw ctx.error("FORBIDDEN", "can only respawn your own avatar");
    }
    await ctx.db.update("Avatar", avatarId, {
      health: 100,
      lastSeenAt: new Date().toISOString(),
    });
    return { health: 100 };
  },
});
