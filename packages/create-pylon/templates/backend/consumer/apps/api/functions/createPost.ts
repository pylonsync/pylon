import { mutation, v } from "@pylonsync/functions";

/**
 * Post something. Resolves the caller's Profile, then writes a Post
 * with `authorId = profile.id`. Returns the inserted row + author
 * card so the client can render it without a follow-up read.
 */
export default mutation({
	args: { body: v.string() },
	async handler(ctx, args: { body: string }) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const body = args.body.trim();
		if (!body) throw ctx.error("EMPTY_BODY", "post body cannot be empty");
		if (body.length > 1000) {
			throw ctx.error("TOO_LONG", "post body capped at 1000 chars");
		}
		const profiles = (await ctx.db.query("Profile", {
			userId: ctx.auth.userId,
		})) as any[];
		if (profiles.length === 0) {
			throw ctx.error(
				"NO_PROFILE",
				"create a profile first via upsertProfile",
			);
		}
		const profile = profiles[0];
		const id = await ctx.db.insert("Post", {
			authorId: profile.id,
			body,
			createdAt: new Date().toISOString(),
		});
		const post = (await ctx.db.get("Post", id)) as any;
		return {
			id: post.id,
			body: post.body,
			createdAt: post.createdAt,
			author: {
				id: profile.id,
				handle: profile.handle,
				displayName: profile.displayName,
			},
			likeCount: 0,
			likedByMe: false,
		};
	},
});
