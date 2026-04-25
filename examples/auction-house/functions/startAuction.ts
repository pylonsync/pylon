import { mutation, v } from "@pylonsync/functions";

/**
 * Flip an auction from "scheduled" to "running". Idempotent — re-runs
 * (e.g. from scheduler retries) are no-ops once the auction is already
 * running or ended.
 */
export default mutation({
  args: {
    auctionId: v.string(),
  },
  async handler(ctx, args) {
    const auction = (await ctx.db.get("Auction", args.auctionId)) as
      | { status: string; kind: string }
      | null;
    if (!auction) return { started: false, reason: "not_found" };
    if (auction.status !== "scheduled") {
      return { started: false, reason: `already_${auction.status}` };
    }
    await ctx.db.update("Auction", args.auctionId, { status: "running" });
    return { started: true };
  },
});
