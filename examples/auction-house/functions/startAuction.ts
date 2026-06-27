import { mutation, v } from "@pylonsync/functions";

/**
 * Flip an auction from "scheduled" to "running". Idempotent — re-runs
 * (e.g. from scheduler retries) are no-ops once the auction is already
 * running or ended.
 */
export default mutation({
  // Scheduler-driven, but declare guest so a guest-session caller is never
  // rejected by the auth: "user" default.
  auth: "guest",
  args: {
    auctionId: v.string(),
  },
  async handler(ctx, args: { auctionId: string }) {
    const auction = (await ctx.db.get("Auction", args.auctionId)) as
      | { status: string; kind: string }
      | null;
    if (!auction) return { started: false, reason: "not_found" };
    if (auction.status !== "scheduled") {
      return { started: false, reason: `already_${auction.status}` };
    }
    await ctx.db.update("Auction", args.auctionId as string, { status: "running" });
    return { started: true };
  },
});
