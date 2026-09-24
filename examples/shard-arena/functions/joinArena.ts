import { action } from "@pylonsync/functions";

/** The one shared arena. */
const SHARD_ID = "arena-main";

/**
 * Start the arena if it is not running, and give the caller a ticket to
 * join it as themselves. An action: starting a shard is not part of a
 * database transaction.
 */
export default action({
  // Anyone with a guest session (POST /api/auth/guest) may play.
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    if (!(await ctx.shards.get(SHARD_ID))) {
      try {
        await ctx.shards.create("arena", SHARD_ID, { width: 800, height: 500 });
      } catch (err) {
        // Two first players at once: the other call created it.
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const ticket = await ctx.shards.ticket(SHARD_ID, { ttlSecs: 300 });
    return { shardId: SHARD_ID, subscriberId: ctx.auth.userId, ticket };
  },
});
