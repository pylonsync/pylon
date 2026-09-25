import { action, v } from "@pylonsync/functions";

/**
 * Move the caller from zone `from` to zone `to`, with their hit points,
 * buffs, and cooldowns. Their connection to `from` gets a transfer frame and
 * the client reconnects to `to`. When `to` refuses (a closed zone), the
 * caller stays in `from` and this throws SHARD_TRANSFER_REFUSED.
 */
export default action({
  auth: "guest",
  args: { from: v.string(), to: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    return ctx.shards.transfer(args.from as string, ctx.auth.userId, args.to as string);
  },
});
