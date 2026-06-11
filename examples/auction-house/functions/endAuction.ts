import { mutation, v } from "@pylonsync/functions";

/**
 * End a live auction: mark any still-running lot as "passed", clear
 * currentLotId, and flip the auction to "ended". Only the auctioneer
 * (auction.creatorId) may call this. Idempotent — re-runs on an
 * already-ended auction are no-ops.
 */
export default mutation({
  // Public demo: anyone with a guest session can be the auctioneer.
  auth: "guest",
  args: {
    auctionId: v.string(),
  },
  async handler(ctx, args: { auctionId: string }) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const auction = (await ctx.db.get("Auction", args.auctionId)) as
      | { kind: string; status: string; creatorId: string; currentLotId?: string | null }
      | null;
    if (!auction) throw ctx.error("AUCTION_NOT_FOUND", "auction not found");
    if (auction.creatorId !== ctx.auth.userId) {
      throw ctx.error("NOT_AUCTIONEER", "only the auctioneer can end this auction");
    }
    if (auction.status === "ended") {
      return { ended: false, reason: "already_ended" };
    }

    // Close the active lot, if any.
    if (auction.currentLotId) {
      const lot = (await ctx.db.get("Lot", auction.currentLotId)) as
        | { status: string; bidCount: number; currentCents: number }
        | null;
      if (lot && lot.status === "running") {
        if (lot.bidCount > 0) {
          // Find the highest bid to record the winner.
          const bids = (await ctx.db.query("Bid", { lotId: auction.currentLotId })) as Array<{
            id: string;
            bidderId: string;
            amountCents: number;
          }>;
          const highest = bids.sort((a, b) => b.amountCents - a.amountCents)[0];
          if (highest) {
            await ctx.db.update("Lot", auction.currentLotId, {
              status: "sold",
              winningBidId: highest.id,
              winnerId: highest.bidderId,
              soldAt: new Date().toISOString(),
            });
          } else {
            await ctx.db.update("Lot", auction.currentLotId, {
              status: "passed",
              soldAt: new Date().toISOString(),
            });
          }
        } else {
          await ctx.db.update("Lot", auction.currentLotId, {
            status: "passed",
            soldAt: new Date().toISOString(),
          });
        }
      }
    }

    await ctx.db.update("Auction", args.auctionId, {
      status: "ended",
      currentLotId: null,
    });

    return { ended: true };
  },
});
