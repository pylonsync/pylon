import { action } from "@pylonsync/functions";

/** The one shared island. */
const SHARD_ID = "island-main";

/**
 * Start the island shard if it is not running, and give the caller a
 * ticket to join it as their avatar. The subscriber id is the Avatar row
 * id, so the shard ties poses and health to the row that holds the name
 * and color. An action: starting a shard is not part of a transaction.
 */
export default action({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const avatar = await ctx.runMutation<{ id: string }>("touchAvatar", {});

    if (!(await ctx.shards.get(SHARD_ID))) {
      try {
        await ctx.shards.create("island", SHARD_ID, {});
      } catch (err) {
        // Two first players at once: the other call created it.
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const ticket = await ctx.shards.ticket(SHARD_ID, {
      subscriberId: avatar.id,
      ttlSecs: 300,
    });
    return { shardId: SHARD_ID, subscriberId: avatar.id, ticket };
  },
});
