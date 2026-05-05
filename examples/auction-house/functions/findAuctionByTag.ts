import { query, v } from "@pylonsync/functions";

/**
 * Internal query for the daily-auction cron's dedup check. Returns
 * any auctions stamped with the given dedupTag (yyyy-mm-dd for the
 * daily cadence). Indexed on `dedupTag` so the lookup stays cheap
 * even after months of dailies accumulate.
 */
export default query({
	args: { tag: v.string() },
	internal: true,
	async handler(ctx, args: { tag: string }) {
		const rows = await ctx.db.query("Auction", {
			dedupTag: args.tag,
			$limit: 1,
		});
		return rows.map((r) => ({ id: r.id as string }));
	},
});
