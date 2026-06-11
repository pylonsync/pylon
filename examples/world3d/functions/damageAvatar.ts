import { mutation, v } from "@pylonsync/functions";

/**
 * Apply weapon damage to another player. The SHOOTER's client detects
 * the hit (capsule raycast) and reports it here; the server is the
 * authority on what actually happens to health:
 *
 *   - damage is clamped to the weapon ceiling (no 9999-damage calls),
 *   - the shooter must have an avatar within weapon range of the
 *     target (a client can't snipe across the map from spawn),
 *   - health floors at 0 — the victim's client observes 0 via the
 *     live query and runs its own death/respawn sequence.
 */
export default mutation({
  auth: "guest",
  args: {
    targetId: v.id("Avatar"),
    amount: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const amount = Math.min(60, Math.max(1, Number(args.amount) || 0));
    const targetId = args.targetId as string;

    const target = await ctx.db.get("Avatar", targetId);
    if (!target) throw ctx.error("NOT_FOUND", "target not found");
    if (target.userId === ctx.auth.userId) {
      return { health: (target.health as number | null) ?? 100, self: true };
    }

    const shooters = await ctx.db.query("Avatar", { userId: ctx.auth.userId });
    const shooter = shooters[0];
    if (!shooter) throw ctx.error("NOT_FOUND", "shooter has no avatar");
    const dx = (shooter.x as number) - (target.x as number);
    const dy = (shooter.y as number) - (target.y as number);
    const dz = (shooter.z as number) - (target.z as number);
    // Weapon range is 220 m; the slack covers grenade throws + the
    // ~10 Hz pose lag between what each client saw and these rows.
    if (Math.hypot(dx, dy, dz) > 280) {
      throw ctx.error("INVALID_ARGS", "target out of weapon range");
    }

    const health = Math.max(0, ((target.health as number | null) ?? 100) - amount);
    // unsafe: deliberately writing ANOTHER player's row. The Avatar
    // update policy is owner-only (so players can't move each other);
    // combat damage is the one sanctioned cross-row write, and every
    // input is server-clamped above.
    await ctx.db.unsafe.update("Avatar", targetId, { health });
    return { health };
  },
});
