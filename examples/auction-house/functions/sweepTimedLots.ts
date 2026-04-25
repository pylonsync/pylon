import { mutation, v } from "@pylonsync/functions";

/**
 * Internal sweeper for timed auctions. Walks all lots in the auction:
 *   - "pending" lots whose `endsAt` has been reached are opened
 *   - "running" lots whose `endsAt` has passed are closed
 *
 * Reschedules itself every 2s while the auction has any open or
 * pending lot left. When everything is resolved the function flips
 * the auction to "ended" and stops rescheduling.
 */
const TICK_MS = 2000;

export default mutation({
  args: {
    auctionId: v.string(),
  },
  async handler(ctx, args) {
    const auction = (await ctx.db.get("Auction", args.auctionId)) as
      | { kind: string; status: string }
      | null;
    if (!auction || auction.kind !== "timed") {
      return { swept: 0, reason: "not_timed" };
    }
    if (auction.status === "ended") {
      return { swept: 0, reason: "ended" };
    }

    if (auction.status === "scheduled") {
      await ctx.db.update("Auction", args.auctionId, { status: "running" });
    }

    const allLots = (await ctx.db.query("Lot")) as Array<{
      id: string;
      auctionId: string;
      status: string;
      endsAt?: string | null;
      bidCount: number;
      currentCents: number;
    }>;
    const lots = allLots.filter((l) => l.auctionId === args.auctionId);
    const now = Date.now();
    let opened = 0;
    let closed = 0;
    let stillActive = 0;

    for (const lot of lots) {
      const endsAt = lot.endsAt ? new Date(lot.endsAt).getTime() : 0;
      if (lot.status === "pending") {
        // Open as soon as we hit its window. Timed lots get their own
        // deadlines via the per-lot endsAt computed at creation.
        if (endsAt <= now) {
          // Already past its own deadline at start — still open it for
          // a brief window so a quick bid can land. 30s minimum window.
          const minEnd = new Date(now + 30_000).toISOString();
          await ctx.db.update("Lot", lot.id, {
            status: "running",
            endsAt: minEnd,
          });
        } else {
          await ctx.db.update("Lot", lot.id, { status: "running" });
        }
        opened++;
        stillActive++;
      } else if (lot.status === "running") {
        if (endsAt > now) {
          stillActive++;
        } else {
          // Time's up — close it.
          const bids = (await ctx.db.query("Bid")) as Array<{
            id: string;
            lotId: string;
            bidderId: string;
            amountCents: number;
          }>;
          const highest = bids
            .filter((b) => b.lotId === lot.id)
            .sort((a, b) => b.amountCents - a.amountCents)[0];
          if (highest) {
            await ctx.db.update("Lot", lot.id, {
              status: "sold",
              winningBidId: highest.id,
              winnerId: highest.bidderId,
              soldAt: new Date().toISOString(),
            });
          } else {
            await ctx.db.update("Lot", lot.id, {
              status: "passed",
              soldAt: new Date().toISOString(),
            });
          }
          closed++;
        }
      }
    }

    if (stillActive > 0) {
      await ctx.scheduler.runAfter(TICK_MS, "sweepTimedLots", {
        auctionId: args.auctionId,
      });
    } else {
      await ctx.db.update("Auction", args.auctionId, { status: "ended" });
    }

    return { opened, closed, stillActive };
  },
});
