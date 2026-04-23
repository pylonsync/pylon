import { mutation, v } from "@pylonsync/functions";

/**
 * Set an avatar's pose. Clients call this at ~10 Hz while moving;
 * the sync engine fans the update to every subscriber. Interpolation
 * on the client absorbs the gap between updates.
 */
export default mutation({
  args: {
    avatarId: v.id("Avatar"),
    x: v.number(),
    y: v.number(),
    z: v.number(),
    heading: v.number(),
    emote: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const row = await ctx.db.get("Avatar", args.avatarId);
    if (!row) throw ctx.error("NOT_FOUND", "avatar not found");

    // Clamp to a 40×40 plane centered at origin.
    const clamp = (v: number) => Math.max(-20, Math.min(20, v));

    await ctx.db.update("Avatar", args.avatarId, {
      x: clamp(args.x),
      y: args.y,
      z: clamp(args.z),
      heading: args.heading,
      emote: args.emote ?? null,
      lastSeenAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
