import { mutation, v } from "@pylonsync/functions";

/**
 * Buy a listing outright at its asking price. Records the purchase as an
 * accepted Offer (so it shows up in both parties' views exactly like a
 * negotiated sale), marks the listing sold, and declines any other pending
 * offers. The cross-entity, multi-row work is why this is a function rather
 * than a plain db.insert.
 */
export default mutation({
  // Defaults to auth: "user" — only signed-in members can buy.
  args: {
    listingId: v.id("Listing"),
    buyerName: v.string(),
    // Threaded by db.useMutation({ optimistic }) so the server's accepted
    // Offer reuses the id of the optimistic ghost the client painted.
    _optimisticId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    const listing = await ctx.db.get("Listing", args.listingId);
    if (!listing) throw ctx.error("NOT_FOUND", "listing not found");
    if (listing.status !== "active")
      throw ctx.error("INVALID_ARGS", "this listing is no longer available");
    if (listing.sellerId === ctx.auth.userId)
      throw ctx.error("INVALID_ARGS", "you can't buy your own listing");

    const id = await ctx.db.insert("Offer", {
      id: args._optimisticId,
      listingId: args.listingId,
      listingTitle: listing.title,
      sellerId: listing.sellerId,
      buyerId: ctx.auth.userId,
      buyerName: args.buyerName || "anonymous",
      amount: listing.price,
      message: "Bought at list price",
      status: "accepted",
      createdAt: new Date().toISOString(),
    });

    // Mark sold and decline the rest — same reconciliation respondToOffer does
    // on an accept.
    await ctx.db.update("Listing", args.listingId, { status: "sold" });
    const siblings = (await ctx.db.list("Offer")).filter(
      (o) =>
        o.listingId === args.listingId &&
        o.id !== id &&
        o.status === "pending",
    );
    for (const o of siblings) {
      await ctx.db.update("Offer", o.id, { status: "declined" });
    }
    return { id };
  },
});
