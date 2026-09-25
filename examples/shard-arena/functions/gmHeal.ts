import { mutation, v } from "@pylonsync/functions";

/**
 * Heal player `userId` in a zone, and log it. The zone gets the heal as a server
 * input (`ctx.shards.send`) after this mutation commits; when it rolls
 * back (`fail`), the zone gets nothing. Admins only.
 */
export default mutation({
  auth: "user",
  args: {
    zone: v.string(),
    userId: v.string(),
    hp: v.number(),
    fail: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.isAdmin) throw ctx.error("FORBIDDEN", "admins only");
    await ctx.db.insert("GmAction", {
      zone: args.zone as string,
      userId: args.userId as string,
      action: `heal ${args.hp}`,
    });
    await ctx.shards.send(args.zone as string, {
      heal: { who: args.userId, hp: args.hp },
    });
    if (args.fail) throw ctx.error("GM_TEST_ROLLBACK", "rolled back on purpose");
    return { sent: true };
  },
});
