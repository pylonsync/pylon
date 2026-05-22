import { query } from "@pylonsync/functions";

/**
 * Live query — every Widget, newest first. Subscribed via the React
 * `useQuery` / Swift `PylonQuery` / RN `useQuery` hooks; rerenders
 * whenever a row is inserted, updated, or deleted.
 */
export default query({
	auth: "public",
	args: {},
	async handler(ctx) {
		const rows = await ctx.db.query("Widget", {});
		return [...rows].sort((a: any, b: any) => {
			const ad = Date.parse(a.createdAt) || 0;
			const bd = Date.parse(b.createdAt) || 0;
			return bd - ad;
		});
	},
});
