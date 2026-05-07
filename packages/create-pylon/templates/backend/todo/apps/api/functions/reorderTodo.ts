import { mutation, v } from "@pylonsync/functions";

/**
 * Drag-reorder. Frontend computes `position` as the midpoint of the
 * drop target's neighbors; we just write it. Floats give us ~52 inserts
 * between any two rows before precision matters.
 */
export default mutation({
	args: { id: v.id("Todo"), position: v.number() },
	async handler(ctx, args: { id: string; position: number }) {
		await ctx.db.update("Todo", args.id, { position: args.position });
		return await ctx.db.get("Todo", args.id);
	},
});
