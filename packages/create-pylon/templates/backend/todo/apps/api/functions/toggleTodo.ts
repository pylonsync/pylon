import { mutation, v } from "@pylonsync/functions";

/**
 * Flip the `done` flag on a Todo. Mutation, not action — needs
 * `ctx.db.update` which is only on writable ctx variants.
 */
export default mutation({
	auth: "public",
	args: { id: v.id("Todo"), done: v.bool() },
	async handler(ctx, args: { id: string; done: boolean }) {
		await ctx.db.update("Todo", args.id, { done: args.done });
		return await ctx.db.get("Todo", args.id);
	},
});
