import { query, v } from "@pylonsync/functions";

/**
 * Public catalogue URL for one auction.
 *
 * Exists to demo a Studio row-action button: `studio.config.ts` declares
 * a `kind: "action"` row action pointed at this function, and Studio
 * copies the returned `url` to the clipboard. That's the shape to copy
 * for any "do one thing to this row" operator task — generate a link,
 * resend an invite, re-run an import — without linking out of Studio to
 * a page you had to build yourself.
 */
export default query({
	// Operators sign into Studio with an account that isn't an app User
	// row, so anything stricter than "guest" would reject them here.
	auth: "guest",
	args: { auctionId: v.string() },
	async handler(ctx, args: { auctionId: string }) {
		const auction = (await ctx.db.get("Auction", args.auctionId)) as {
			title: string;
		} | null;
		if (!auction) throw ctx.error("AUCTION_NOT_FOUND", "auction not found");
		const base = process.env.PYLON_PUBLIC_URL ?? "http://localhost:4321";
		return {
			title: auction.title,
			url: `${base.replace(/\/$/, "")}/auctions/${args.auctionId}`,
		};
	},
});
