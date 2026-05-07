import { mutation, v } from "@pylonsync/functions";

/**
 * Toggle a like on a Post. Idempotent — calling twice from the same
 * user lands at the original state. Returns { liked: boolean,
 * likeCount: number } so the client can update its UI without a
 * follow-up read.
 */
export default mutation({
	args: { postId: v.id("Post") },
	async handler(ctx, args: { postId: string }) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const profiles = (await ctx.db.query("Profile", {
			userId: ctx.auth.userId,
		})) as any[];
		if (profiles.length === 0) {
			throw ctx.error("NO_PROFILE", "create a profile first");
		}
		const profile = profiles[0];

		const existing = (await ctx.db.query("Like", {
			postId: args.postId,
			profileId: profile.id,
		})) as any[];

		if (existing.length === 0) {
			await ctx.db.insert("Like", {
				postId: args.postId,
				profileId: profile.id,
				createdAt: new Date().toISOString(),
			});
		} else {
			for (const like of existing) {
				await ctx.db.delete("Like", like.id);
			}
		}

		const allLikes = (await ctx.db.query("Like", {
			postId: args.postId,
		})) as any[];
		return {
			liked: existing.length === 0,
			likeCount: allLikes.length,
		};
	},
});
