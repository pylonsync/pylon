import { query } from "@statecraft/functions";

export default query({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) return [];
    const bids = await ctx.db.query("Bid", {
      bidderId: ctx.auth.userId,
      $order: "createdAt desc",
      $limit: 50,
    });
    const out = [];
    for (const bid of bids) {
      const listing = await ctx.db.get("Listing", bid.listingId as string);
      out.push({ ...bid, listing });
    }
    return out;
  },
});
