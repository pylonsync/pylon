import { action, v } from "@pylonsync/functions";

/** The shared frontier, when the caller names none. */
const DEFAULT_FRONTIER = "frontier-main";

/**
 * Start a frontier zone if it is not running, and give the caller a ticket
 * to join it as themselves. Same shape as joinArena.
 *
 * A player sees 250 units around them and gets at most 900 bytes of updates
 * per tick (18 KB/s at 20 Hz). `size` sets the side of a NEW zone, default
 * 2000; a small zone puts every player in view of the others, like a keep
 * fight.
 */
export default action({
  // Anyone with a guest session (POST /api/auth/guest) may play.
  auth: "guest",
  args: {
    frontier: v.optional(v.string()),
    size: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const shardId = (args.frontier as string | undefined) ?? DEFAULT_FRONTIER;
    const size = (args.size as number | undefined) ?? 2000;
    if (!(size >= 100 && size <= 20000)) {
      throw ctx.error("INVALID_ARGS", "size must be between 100 and 20000");
    }
    if (!(await ctx.shards.get(shardId))) {
      try {
        await ctx.shards.create("frontier", shardId, {
          width: size,
          height: size,
          view_radius: 250,
          max_bytes_per_tick: 900,
        });
      } catch (err) {
        // Two first players at once: the other call created it.
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const ticket = await ctx.shards.ticket(shardId, { ttlSecs: 300 });
    return { shardId, subscriberId: ctx.auth.userId, ticket };
  },
});
