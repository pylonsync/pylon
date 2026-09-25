import { action, v } from "@pylonsync/functions";

/**
 * Start zone `zone` if it is not running (on `machine`, when given), and give
 * the caller a ticket to join it as themselves.
 *
 * `params` reach the zone's init: `{ closed: true }` refuses players moving
 * in; `{ edge, next }` moves a player whose x reaches `edge` to zone `next`.
 */
export default action({
  auth: "guest",
  args: {
    zone: v.string(),
    machine: v.optional(v.string()),
    params: v.optional(v.any()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    const zone = args.zone as string;
    if (!(await ctx.shards.get(zone))) {
      try {
        await ctx.shards.create("zone", zone, args.params ?? {}, {
          machine: args.machine as string | undefined,
        });
      } catch (err) {
        if ((err as { code?: string }).code !== "SHARD_EXISTS") throw err;
      }
    }
    const info = await ctx.shards.get(zone);
    const ticket = await ctx.shards.ticket(zone, { ttlSecs: 300 });
    return { zone, subscriberId: ctx.auth.userId, ticket, machine: info?.machine };
  },
});
