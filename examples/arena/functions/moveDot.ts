import { mutation, v } from "@pylonsync/functions";

/**
 * Set a dot's target position. Also writes the current (x,y) — the
 * client passes its best-known position so every replica snaps to the
 * same authoritative state after interpolation.
 *
 * Pylon's change-log fans this single write out to every subscriber
 * (live query on Dot) in a single server round-trip. Hot path: ~2KB
 * WS frame per broadcast, one IO per mutation.
 */
export default mutation({
  args: {
    dotId: v.id("Dot"),
    x: v.number(),
    y: v.number(),
    tx: v.number(),
    ty: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const dot = await ctx.db.get("Dot", args.dotId);
    if (!dot) throw ctx.error("NOT_FOUND", "dot not found");

    // Clamp to [0,1] so the client can't send nonsense.
    const clamp = (v: number) => Math.max(0, Math.min(1, v));

    await ctx.db.update("Dot", args.dotId, {
      x: clamp(args.x),
      y: clamp(args.y),
      tx: clamp(args.tx),
      ty: clamp(args.ty),
      lastSeenAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
