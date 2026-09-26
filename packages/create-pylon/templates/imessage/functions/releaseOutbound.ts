import { mutation, v } from "@pylonsync/functions";

// Return leased Sendblue messages to the queue after an earlier message in the
// same batch failed. Internal: deliverOutbound only.
export default mutation({
  internal: true,
  auth: "admin",
  args: { leaseToken: v.string(), messageIds: v.array(v.string()) },
  async handler(ctx, args) {
    for (const id of args.messageIds) {
      const row = await ctx.db.get("Message", id);
      if (row?.status === "leased" && row.leaseToken === args.leaseToken) {
        await ctx.db.update("Message", id, { status: "queued", leaseUntil: null, leaseToken: null });
      }
    }
    return { released: args.messageIds.length };
  },
});
