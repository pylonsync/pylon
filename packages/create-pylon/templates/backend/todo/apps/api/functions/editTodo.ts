import { mutation, v } from "@pylonsync/functions";

/**
 * Rename a Todo. Trims whitespace; rejects empty titles.
 */
export default mutation({
	auth: "public",
	args: { id: v.id("Todo"), title: v.string() },
	async handler(ctx, args: { id: string; title: string }) {
		const trimmed = args.title.trim();
		if (!trimmed) {
			throw ctx.error("EMPTY_TITLE", "title cannot be empty");
		}
		await ctx.db.update("Todo", args.id, { title: trimmed });
		return await ctx.db.get("Todo", args.id);
	},
});
