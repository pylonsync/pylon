import { mutation, v } from "@pylonsync/functions";

/**
 * Make an offer on a listing. Denormalizes the listing title + seller id
 * onto the offer so the seller's inbox + the buyer's "my offers" list can
 * render without a join.
 */
export default mutation({
  auth: "guest",
  args: {
    listingId: v.id("Listing"),
    amount: v.number(),
    message: v.optional(v.string()),
    buyerName: v.string(),
    // Threaded by db.useMutation({ optimistic }) so the server row reuses the
    // id of the optimistic ghost the client already painted — the broadcast
    // merges in place instead of flashing a temp row.
    _optimisticId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    const listing = await ctx.db.get("Listing", args.listingId);
    if (!listing) throw ctx.error("NOT_FOUND", "listing not found");
    if (listing.status !== "active")
      throw ctx.error("INVALID_ARGS", "this listing is no longer available");
    if (listing.sellerId === ctx.auth.userId)
      throw ctx.error("INVALID_ARGS", "you can't bid on your own listing");
    if (args.amount <= 0)
      throw ctx.error("INVALID_ARGS", "offer must be greater than zero");

    const id = await ctx.db.insert("Offer", {
      // Reuse the optimistic ghost's id so the broadcast merges in place.
      id: args._optimisticId,
      listingId: args.listingId,
      listingTitle: listing.title,
      sellerId: listing.sellerId,
      buyerId: ctx.auth.userId,
      buyerName: args.buyerName || "anonymous",
      amount: Math.round(args.amount * 100) / 100,
      message: (args.message ?? "").trim(),
      status: "pending",
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
