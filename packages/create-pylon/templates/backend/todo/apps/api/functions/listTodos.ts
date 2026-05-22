import { query } from "@pylonsync/functions";

/**
 * Live query — every Todo, in user-controlled drag-reorder position.
 * Rows without a `position` (legacy data) get sorted by createdAt as
 * a fallback so the list stays deterministic.
 */
export default query({
	auth: "public",
	args: {},
	async handler(ctx) {
		const rows = await ctx.db.query("Todo", {});
		return [...rows].sort((a: any, b: any) => {
			const ap =
				typeof a.position === "number"
					? a.position
					: Date.parse(a.createdAt) || 0;
			const bp =
				typeof b.position === "number"
					? b.position
					: Date.parse(b.createdAt) || 0;
			return ap - bp;
		});
	},
});
