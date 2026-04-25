import { mutation, v } from "@pylonsync/functions";

/**
 * Atomic bid: validate against current price + min increment, write
 * the Bid row, bump the lot's current price/bidCount, and (for live
 * auctions) bump the lot's antishill timer.
 *
 * The auction-status check rejects bids on lots whose parent auction
 * isn't running, so a "scheduled" auction can be inspected but not
 * sniped before its start time.
 */
const ANTISHILL_RESET_SECS = 12;

export default mutation({
  args: {
    lotId: v.string(),
    amountCents: v.int(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to bid");
    if (args.amountCents <= 0) {
      throw ctx.error("BAD_AMOUNT", "bid must be positive");
    }

    // Guest sessions have a userId but no User row; password-registered
    // users do. Either way we display a name on the Bid row, falling
    // back to a stable Guest-XXXX label so the bid stream is readable.
    const bidder = (await ctx.db.get("User", ctx.auth.userId)) as
      | { displayName?: string; balanceCents?: number | null }
      | null;
    const bidderName =
      bidder?.displayName ?? `Guest-${ctx.auth.userId.slice(-6)}`;
    // For the demo, treat a missing/null balance as effectively
    // unlimited — there's no deposit flow yet.
    const balance = bidder?.balanceCents ?? 10_000_000_00;
    if (balance < args.amountCents) {
      throw ctx.error(
        "INSUFFICIENT_FUNDS",
        `bid exceeds your balance of $${(balance / 100).toFixed(2)}`,
      );
    }

    const lot = (await ctx.db.get("Lot", args.lotId)) as
      | {
          auctionId: string;
          status: string;
          currentCents: number;
          minIncrementCents: number;
          bidCount: number;
          endsAt?: string | null;
        }
      | null;
    if (!lot) throw ctx.error("LOT_NOT_FOUND", "lot does not exist");
    if (lot.status !== "running") {
      throw ctx.error("LOT_NOT_OPEN", `lot is ${lot.status}`);
    }

    const auction = (await ctx.db.get("Auction", lot.auctionId)) as
      | { status: string; kind: string }
      | null;
    if (!auction || auction.status !== "running") {
      throw ctx.error("AUCTION_NOT_RUNNING", "auction is not accepting bids");
    }

    const minBid = lot.currentCents + lot.minIncrementCents;
    if (args.amountCents < minBid && lot.bidCount > 0) {
      throw ctx.error(
        "BID_TOO_LOW",
        `must be at least $${(minBid / 100).toFixed(2)}`,
      );
    }
    if (args.amountCents < lot.currentCents && lot.bidCount === 0) {
      throw ctx.error(
        "BELOW_RESERVE",
        `starting bid is $${(lot.currentCents / 100).toFixed(2)}`,
      );
    }

    const now = new Date();
    const bidId = await ctx.db.insert("Bid", {
      auctionId: lot.auctionId,
      lotId: args.lotId,
      bidderId: ctx.auth.userId,
      bidderName,
      amountCents: args.amountCents,
      createdAt: now.toISOString(),
    });

    // For a live auction lot, every fresh bid bumps the deadline so a
    // last-second snipe can be answered. For timed auctions the deadline
    // is fixed when the auction is created.
    let nextEndsAt = lot.endsAt;
    if (auction.kind === "live" && lot.endsAt) {
      const reset = new Date(now.getTime() + ANTISHILL_RESET_SECS * 1000);
      const current = new Date(lot.endsAt);
      if (reset > current) {
        nextEndsAt = reset.toISOString();
      }
    }

    await ctx.db.update("Lot", args.lotId, {
      currentCents: args.amountCents,
      bidCount: lot.bidCount + 1,
      endsAt: nextEndsAt,
    });

    return { bidId, currentCents: args.amountCents };
  },
});
