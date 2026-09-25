import { action, v } from "@pylonsync/functions";

/**
 * Send `text` to zones on any machine, as a server message: `to` is "all"
 * (default), "group:<name>", or "shard:<id>". Each zone lists it among the
 * shouts it heard.
 */
export default action({
  auth: "guest",
  args: { text: v.string(), to: v.optional(v.string()) },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");
    await ctx.shards.publish((args.to as string | undefined) ?? "all", "shout", args.text);
    return { sent: true };
  },
});
