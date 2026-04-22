import { mutation, v } from "@statecraft/functions";

export default mutation({
  args: { listingId: v.id("Listing") },
  async handler(ctx, args) {
    const listing = await ctx.db.get("Listing", args.listingId);
    if (!listing) return { settled: false, reason: "not_found" };
    if (listing.settledAt) return { settled: false, reason: "already_settled" };
    if (new Date(listing.endsAt).getTime() > Date.now()) {
      return { settled: false, reason: "not_ended" };
    }

    const now = new Date().toISOString();

    if (!listing.winningBidId) {
      await ctx.db.update("Listing", args.listingId, { settledAt: now });
      return { settled: true, winnerId: null };
    }

    const bid = await ctx.db.get("Bid", listing.winningBidId);
    if (!bid) {
      await ctx.db.update("Listing", args.listingId, { settledAt: now });
      return { settled: true, winnerId: null };
    }

    const winner = await ctx.db.get("User", bid.bidderId);
    const seller = await ctx.db.get("User", listing.sellerId);
    if (!winner || !seller) throw ctx.error("INVARIANT", "missing user");

    if (winner.balanceCents < bid.amountCents) {
      await ctx.db.update("Listing", args.listingId, {
        settledAt: now,
        winningBidId: null,
      });
      return { settled: true, winnerId: null, reason: "insolvent_winner" };
    }

    await ctx.db.update("User", winner.id, {
      balanceCents: winner.balanceCents - bid.amountCents,
    });
    await ctx.db.update("User", seller.id, {
      balanceCents: seller.balanceCents + bid.amountCents,
    });
    await ctx.db.update("Listing", args.listingId, { settledAt: now });

    return { settled: true, winnerId: winner.id, paidCents: bid.amountCents };
  },
});
