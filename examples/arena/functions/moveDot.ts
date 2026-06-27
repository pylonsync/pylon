import { mutation, v } from "@pylonsync/functions";

/**
 * Set a dot's target position. Also writes the current (x,y) — the
 * client passes its best-known position so every replica snaps to the
 * same authoritative state after interpolation.
 *
 * Pylon's change-log fans this single write out to every subscriber
 * (live query on Dot) in a single server round-trip.
 */
export default mutation({
  // Public demo: guest sessions drive the simulation. Defaults to
  // auth: "user" (rejecting guests) without this.
  auth: "guest",
  args: {
    dotId: v.id("Dot"),
    x: v.number(),
    y: v.number(),
    tx: v.number(),
    ty: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const dot = await ctx.db.get("Dot", args.dotId as string);
    if (!dot) throw ctx.error("NOT_FOUND", "dot not found");

    // Clamp to [0,1] so the client can't send nonsense.
    const clamp = (n: number) => Math.max(0, Math.min(1, n));

    await ctx.db.update("Dot", args.dotId as string, {
      x: clamp(args.x as number),
      y: clamp(args.y as number),
      tx: clamp(args.tx as number),
      ty: clamp(args.ty as number),
      lastSeenAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
