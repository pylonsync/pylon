import { mutation, v } from "@pylonsync/functions";
import { requireProfile } from "../lib/social";

/** Delete one of the caller's posts with its likes and comments. */
export default mutation({
	args: { id: v.id("Post") },
	async handler(ctx, args: { id: string }) {
		const profile = await requireProfile(ctx, ctx.auth.userId);
		const post = await ctx.db.get("Post", args.id);
		if (!post) throw ctx.error("NOT_FOUND", "This post no longer exists.");
		if (post.authorId !== profile.id) {
			throw ctx.error("FORBIDDEN", "You can only delete your own posts.");
		}
		for (const like of await ctx.db.query("Like", { postId: args.id })) {
			await ctx.db.unsafe.delete("Like", String(like.id));
		}
		for (const comment of await ctx.db.query("Comment", { postId: args.id })) {
			await ctx.db.unsafe.delete("Comment", String(comment.id));
		}
		await ctx.db.unsafe.delete("Post", args.id);
		return { id: args.id };
	},
});
