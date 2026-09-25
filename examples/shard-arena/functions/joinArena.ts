import { action, v } from "@pylonsync/functions";

/** The shared arena, when the caller names none. */
const DEFAULT_ARENA = "arena-main";

/**
 * Start an arena if it is not running, and give the caller a ticket to join
 * it as themselves. An action: starting a shard is not part of a database
 * transaction.
 *
 * `arena` names another arena (a shard per arena). On an app that runs on
 * several machines, `machine` places a new arena on that machine; without
 * it the arena goes to the machine with the most free capacity.
 */
export default action({
  // Anyone with a guest session (POST /api/auth/guest) may play.
  auth: "guest",
  args: {
    arena: v.optional(v.string()),
    machine: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const shardId = (args.arena as string | undefined) ?? DEFAULT_ARENA;
    const existing = await ctx.shards.get(shardId);
    let machine = existing?.machine;
    if (!existing) {
      try {
        const info = await ctx.shards.create(
          "arena",
          shardId,
          { width: 800, height: 500 },
          { machine: args.machine as string | undefined },
        );
        machine = info.machine;
      } catch (err) {
        // Two first players at once: the other call created it.
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const ticket = await ctx.shards.ticket(shardId, { ttlSecs: 300 });
    return { shardId, subscriberId: ctx.auth.userId, ticket, machine };
  },
});
