import { query } from "@pylonsync/functions";

/**
 * Internal query: return the oldest User by createdAt. Used by the
 * daily-auction cron to pick a system "auctioneer" creator id for
 * auto-generated auctions without polluting a real user's profile.
 *
 * Returns an empty array if no users exist yet (cron defers and
 * retries on its next tick).
 */
export default query({
	// Internal already blocks client calls; declare guest so the post-v0.3.256
	// auth: "user" default never rejects a guest-session caller.
	auth: "guest",
	args: {},
	internal: true,
	async handler(ctx) {
		const rows = await ctx.db.query("User", {
			$order: { createdAt: "asc" },
			$limit: 1,
		});
		return rows.map((r) => ({ id: r.id as string }));
	},
});
