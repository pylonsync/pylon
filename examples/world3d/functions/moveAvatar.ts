import { mutation, v } from "@pylonsync/functions";

/**
 * Set an avatar's pose. Clients call this at ~10 Hz while moving;
 * the sync engine fans the update to every subscriber. Interpolation
 * on the client absorbs the gap between updates.
 */
export default mutation({
  auth: "guest",
  args: {
    avatarId: v.id("Avatar"),
    x: v.number(),
    y: v.number(),
    z: v.number(),
    heading: v.number(),
    pitch: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const avatarId = args.avatarId as string;
    const row = await ctx.db.get("Avatar", avatarId);
    if (!row) throw ctx.error("NOT_FOUND", "avatar not found");

    // Clamp to the island bounds (world is 512×512 centered at origin).
    const clamp = (n: number) => Math.max(-250, Math.min(250, n));

    await ctx.db.update("Avatar", avatarId, {
      x: clamp(args.x as number),
      y: Math.max(-10, Math.min(200, args.y as number)),
      z: clamp(args.z as number),
      heading: args.heading as number,
      pitch: Math.max(-1.55, Math.min(1.55, args.pitch as number)),
      lastSeenAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
