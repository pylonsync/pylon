import { mutation, v } from "@pylonsync/functions";

/**
 * Insert a new Todo. Seeds `position` to (max + 1024) so new rows
 * land at the end of the drag-reorder list; the 1024 step leaves
 * room for inserts-between without needing global renumber.
 */
export default mutation({
	args: { title: v.string() },
	async handler(ctx, args: { title: string }) {
		const existing = await ctx.db.query("Todo", {});
		const maxPos = existing.reduce((acc: number, row: any) => {
			const p =
				typeof row.position === "number"
					? row.position
					: Date.parse(row.createdAt) || 0;
			return p > acc ? p : acc;
		}, 0);
		const id = await ctx.db.insert("Todo", {
			title: args.title,
			done: false,
			createdAt: new Date().toISOString(),
			position: maxPos + 1024,
		});
		return await ctx.db.get("Todo", id);
	},
});
