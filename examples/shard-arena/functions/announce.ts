import { action, v } from "@pylonsync/functions";

/**
 * Send `text` to zones on any machine, as a server message: `to` is "all"
 * (default), "group:<name>", or "shard:<id>". Each zone lists it among the
 * shouts it heard. Admins only: a zone cannot tell a server message from
 * a GM's, so a player must not be able to send one.
 */
export default action({
  auth: "user",
  args: { text: v.string(), to: v.optional(v.string()) },
  async handler(ctx, args) {
    if (!ctx.auth.isAdmin) throw ctx.error("FORBIDDEN", "admins only");
    await ctx.shards.publish((args.to as string | undefined) ?? "all", "shout", args.text);
    return { sent: true };
  },
});
