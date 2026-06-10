import { mutation, v } from "@pylonsync/functions";

/**
 * DB-side half of the daily-auction cron. Inserts an Auction row
 * stamped with `dedupTag` plus N Lot rows in one mutation. Kept
 * separate from the `dailyAuctionCron` action because actions can't
 * touch ctx.db directly — only via runMutation.
 *
 * `kind` is hard-coded to "timed" because the daily cadence implies
 * an unattended auction (no live auctioneer to drive lot transitions).
 */
export default mutation({
	// Internal already blocks client calls; declare guest so the post-v0.3.256
	// auth: "user" default never rejects a guest-session caller.
	auth: "guest",
	args: {
		title: v.string(),
		description: v.string(),
		creatorId: v.id("User"),
		startsAt: v.string(),
		endsAt: v.string(),
		bannerColor: v.string(),
		dedupTag: v.string(),
		lots: v.array(
			v.object({
				title: v.string(),
				description: v.string(),
				imageColor: v.string(),
				startingCents: v.int(),
				minIncrementCents: v.int(),
				endsAt: v.string(),
			}),
		),
	},
	internal: true,
	async handler(ctx, args) {
		const now = new Date().toISOString();
		const auctionId = await ctx.db.insert("Auction", {
			title: args.title,
			description: args.description,
			kind: "timed",
			status: "scheduled",
			creatorId: args.creatorId,
			startsAt: args.startsAt,
			endsAt: args.endsAt,
			bannerColor: args.bannerColor,
			dedupTag: args.dedupTag,
			createdAt: now,
		});
		for (let i = 0; i < args.lots.length; i++) {
			const lot = args.lots[i];
			await ctx.db.insert("Lot", {
				auctionId,
				position: i,
				title: lot.title,
				description: lot.description,
				imageColor: lot.imageColor,
				startingCents: lot.startingCents,
				currentCents: lot.startingCents,
				minIncrementCents: lot.minIncrementCents,
				bidCount: 0,
				status: "pending",
				endsAt: lot.endsAt,
				createdAt: now,
			});
		}
		return auctionId;
	},
});
