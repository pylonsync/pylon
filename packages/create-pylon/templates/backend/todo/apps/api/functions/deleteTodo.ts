import { mutation, v } from "@pylonsync/functions";

/**
 * Remove a Todo row. Returns the row as it existed pre-delete so
 * the client can show a "todo removed" toast or animate it out.
 */
export default mutation({
	args: { id: v.id("Todo") },
	async handler(ctx, args: { id: string }) {
		const snapshot = await ctx.db.get("Todo", args.id);
		await ctx.db.delete("Todo", args.id);
		return snapshot;
	},
});
