import { action, v } from "@pylonsync/functions";

/**
 * Start a shooting range if it is not running, and give the caller a
 * ticket to it. `history` turns lag compensation on (ticks of history to
 * keep, for example 10); a range's settings are fixed when it starts, so
 * ranges with and without history need different names.
 */
export default action({
  auth: "guest",
  args: {
    range: v.optional(v.string()),
    history: v.optional(v.number()),
    speed: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const history = (args.history as number | undefined) ?? 0;
    const shardId = (args.range as string | undefined) ?? `range-${history}`;
    if (!(await ctx.shards.get(shardId))) {
      try {
        await ctx.shards.create("range", shardId, { history, speed: args.speed ?? 8 });
      } catch (err) {
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const ticket = await ctx.shards.ticket(shardId, { ttlSecs: 300 });
    return { shardId, subscriberId: ctx.auth.userId, ticket };
  },
});
