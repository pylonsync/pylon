import { mutation } from "@pylonsync/functions";

/**
 * The caller's avatar id, with its lastSeenAt set to now (spawnAvatar
 * prunes avatars nobody has used for a day). Internal: joinIsland calls
 * it with the caller's auth.
 */
export default mutation({
  internal: true,
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const rows = await ctx.db.query("Avatar", { userId: ctx.auth.userId });
    const avatar = rows[0];
    if (!avatar) throw ctx.error("NOT_FOUND", "spawn an avatar first");
    await ctx.db.update("Avatar", avatar.id as string, {
      lastSeenAt: new Date().toISOString(),
    });
    return { id: avatar.id as string };
  },
});
