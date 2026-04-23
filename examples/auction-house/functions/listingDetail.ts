import { query, v } from "@pylonsync/functions";

export default query({
  args: { listingId: v.id("Listing") },
  async handler(ctx, args) {
    const listing = await ctx.db.get("Listing", args.listingId);
    if (!listing) return null;

    const bids = await ctx.db.query("Bid", {
      listingId: args.listingId,
      $order: "createdAt desc",
      $limit: 20,
    });
    const seller = await ctx.db.get("User", listing.sellerId);

    return { listing, seller, bids };
  },
});
