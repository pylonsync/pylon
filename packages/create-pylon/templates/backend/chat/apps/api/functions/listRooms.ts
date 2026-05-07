import { query } from "@pylonsync/functions";

/**
 * Every room, oldest first (so the General room created on first run
 * stays at the top regardless of activity).
 */
export default query({
	args: {},
	async handler(ctx) {
		const rows = (await ctx.db.query("Room", {})) as any[];
		return rows.sort(
			(a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt),
		);
	},
});
