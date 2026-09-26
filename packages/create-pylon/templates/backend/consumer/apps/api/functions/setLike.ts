import { mutation, v } from "@pylonsync/functions";
import { requireProfile } from "../lib/social";

/**
 * Like or unlike a post as the caller. Idempotent: `liked: true` twice
 * leaves one like, so taps that reach the server out of order still end
 * in the state of the last tap. Returns the post's like count.
 */
export default mutation({
	args: { postId: v.id("Post"), liked: v.boolean() },
	async handler(ctx, args: { postId: string; liked: boolean }) {
		const profile = await requireProfile(ctx, ctx.auth.userId);
		const post = await ctx.db.get("Post", args.postId);
		if (!post) throw ctx.error("NOT_FOUND", "This post no longer exists.");

		await ctx.db.advisoryLock(`like:${profile.id}:${args.postId}`);
		const existing = await ctx.db.query("Like", {
			postId: args.postId,
			profileId: profile.id,
		});
		if (args.liked && existing.length === 0) {
			await ctx.db.unsafe.insert("Like", {
				postId: args.postId,
				profileId: profile.id,
				createdAt: new Date().toISOString(),
			});
		}
		if (!args.liked) {
			for (const like of existing) {
				await ctx.db.unsafe.delete("Like", String(like.id));
			}
		}
		const likes = await ctx.db.query("Like", { postId: args.postId });
		return { liked: args.liked, likeCount: likes.length };
	},
});
