import { mutation, v } from "@statecraft/functions";

const MIN_INCREMENT_CENTS = 100;

export default mutation({
  args: {
    listingId: v.id("Listing"),
    amountCents: v.number(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to bid");

    const listing = await ctx.db.get("Listing", args.listingId);
    if (!listing) throw ctx.error("NOT_FOUND", "listing not found");

    if (listing.sellerId === ctx.auth.userId) {
      throw ctx.error("INVALID", "cannot bid on your own listing");
    }
    if (listing.settledAt) {
      throw ctx.error("INVALID", "auction has already settled");
    }
    if (new Date(listing.endsAt).getTime() <= Date.now()) {
      throw ctx.error("INVALID", "auction has ended");
    }
    if (args.amountCents < listing.currentCents + MIN_INCREMENT_CENTS) {
      throw ctx.error(
        "INVALID",
        `bid must be at least ${listing.currentCents + MIN_INCREMENT_CENTS} cents`,
      );
    }

    const bidder = await ctx.db.get("User", ctx.auth.userId);
    if (!bidder) throw ctx.error("NOT_FOUND", "user record missing");
    if (bidder.balanceCents < args.amountCents) {
      throw ctx.error("INVALID", "insufficient balance");
    }

    const bidId = await ctx.db.insert("Bid", {
      listingId: args.listingId,
      bidderId: ctx.auth.userId,
      amountCents: args.amountCents,
      createdAt: new Date().toISOString(),
    });

    await ctx.db.update("Listing", args.listingId, {
      currentCents: args.amountCents,
      winningBidId: bidId,
    });

    return { bidId, currentCents: args.amountCents };
  },
});
