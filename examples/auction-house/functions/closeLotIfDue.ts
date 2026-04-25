import { mutation, v } from "@pylonsync/functions";

/**
 * Internal: scheduled check that closes a lot if its `endsAt` has
 * passed. If a recent bid bumped the deadline (live auction antishill),
 * reschedules itself for the new deadline. Idempotent on lots that are
 * already sold/passed.
 */
export default mutation({
  args: {
    lotId: v.string(),
  },
  async handler(ctx, args) {
    const lot = (await ctx.db.get("Lot", args.lotId)) as
      | {
          auctionId: string;
          status: string;
          endsAt?: string | null;
          bidCount: number;
          currentCents: number;
        }
      | null;
    if (!lot) return { closed: false, reason: "not_found" };
    if (lot.status !== "running") {
      return { closed: false, reason: `already_${lot.status}` };
    }
    const now = Date.now();
    const ends = lot.endsAt ? new Date(lot.endsAt).getTime() : 0;
    if (ends > now) {
      // Bid bumped the deadline; reschedule.
      await ctx.scheduler.runAfter(Math.max(1000, ends - now + 200), "closeLotIfDue", {
        lotId: args.lotId,
      });
      return { closed: false, reason: "deadline_extended" };
    }

    // Find the highest bid for this lot.
    const bids = (await ctx.db.query("Bid")) as Array<{
      id: string;
      lotId: string;
      bidderId: string;
      amountCents: number;
    }>;
    const highest = bids
      .filter((b) => b.lotId === args.lotId)
      .sort((a, b) => b.amountCents - a.amountCents)[0];

    if (highest) {
      await ctx.db.update("Lot", args.lotId, {
        status: "sold",
        winningBidId: highest.id,
        winnerId: highest.bidderId,
        soldAt: new Date().toISOString(),
      });
    } else {
      await ctx.db.update("Lot", args.lotId, {
        status: "passed",
        soldAt: new Date().toISOString(),
      });
    }

    // Auctioneer-driven live auction: if this was the current lot,
    // clear the auction's currentLotId so the UI shows "between lots".
    const auction = (await ctx.db.get("Auction", lot.auctionId)) as
      | { kind: string; currentLotId?: string | null }
      | null;
    if (auction && auction.currentLotId === args.lotId) {
      await ctx.db.update("Auction", lot.auctionId, { currentLotId: null });
    }

    return { closed: true, status: highest ? "sold" : "passed" };
  },
});
