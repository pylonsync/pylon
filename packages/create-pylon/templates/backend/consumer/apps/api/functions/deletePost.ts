import { mutation, v } from "@pylonsync/functions";

/**
 * Delete a Post and any associated Likes. Post policy already gates
 * deletes to the author's Profile, so a non-author can't reach the
 * actual delete call — but we double-check so a future policy
 * loosening doesn't accidentally orphan Like rows.
 */
export default mutation({
	args: { id: v.id("Post") },
	async handler(ctx, args: { id: string }) {
		const snapshot = (await ctx.db.get("Post", args.id)) as any;
		if (!snapshot) throw ctx.error("NOT_FOUND", "post not found");
		const likes = (await ctx.db.query("Like", { postId: args.id })) as any[];
		for (const like of likes) {
			await ctx.db.delete("Like", like.id);
		}
		await ctx.db.delete("Post", args.id);
		return snapshot;
	},
});
