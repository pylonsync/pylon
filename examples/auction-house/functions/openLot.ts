import { mutation, v } from "@pylonsync/functions";

/**
 * Live-auction control: the auctioneer opens a specific lot for
 * bidding. Sets the antishill clock (initial 30s, sliding) and marks
 * the lot as the auction's `currentLotId`. Closes any previously
 * running lot in the same auction first.
 */
const INITIAL_LIVE_WINDOW_SECS = 30;

export default mutation({
  args: {
    lotId: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const lot = (await ctx.db.get("Lot", args.lotId)) as
      | { auctionId: string; status: string }
      | null;
    if (!lot) throw ctx.error("LOT_NOT_FOUND", "lot does not exist");

    const auction = (await ctx.db.get("Auction", lot.auctionId)) as
      | { kind: string; status: string; creatorId: string; currentLotId?: string | null }
      | null;
    if (!auction) throw ctx.error("AUCTION_NOT_FOUND", "");
    if (auction.kind !== "live") {
      throw ctx.error(
        "NOT_LIVE",
        "openLot only applies to live auctions; timed lots open automatically",
      );
    }
    if (auction.creatorId !== ctx.auth.userId) {
      throw ctx.error("NOT_AUCTIONEER", "only the auctioneer can open lots");
    }
    if (auction.status !== "running") {
      // First lot opens the auction; mark it running and continue.
      await ctx.db.update("Auction", lot.auctionId, { status: "running" });
    }

    // Close the previously running lot, if any.
    if (auction.currentLotId && auction.currentLotId !== args.lotId) {
      await ctx.db.update("Lot", auction.currentLotId, { status: "passed" });
    }

    const endsAt = new Date(
      Date.now() + INITIAL_LIVE_WINDOW_SECS * 1000,
    ).toISOString();
    await ctx.db.update("Lot", args.lotId, {
      status: "running",
      endsAt,
    });
    await ctx.db.update("Auction", lot.auctionId, {
      currentLotId: args.lotId,
    });

    // Schedule a deadline check. The check reschedules itself if the
    // antishill timer has been pushed by a fresh bid.
    await ctx.scheduler.runAfter(INITIAL_LIVE_WINDOW_SECS * 1000, "closeLotIfDue", {
      lotId: args.lotId,
    });

    return { opened: true, endsAt };
  },
});
