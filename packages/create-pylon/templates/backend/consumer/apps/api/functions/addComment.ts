import { mutation, v } from "@pylonsync/functions";
import { COMMENT_MAX, requireProfile } from "../lib/social";

/** Add a comment to a post as the caller. */
export default mutation({
	args: { postId: v.id("Post"), text: v.string() },
	async handler(ctx, args: { postId: string; text: string }) {
		const profile = await requireProfile(ctx, ctx.auth.userId);
		const text = args.text.trim();
		if (!text) throw ctx.error("EMPTY_COMMENT", "Write a comment first.");
		if (text.length > COMMENT_MAX) {
			throw ctx.error("COMMENT_TOO_LONG", `Use at most ${COMMENT_MAX} characters.`);
		}
		const post = await ctx.db.get("Post", args.postId);
		if (!post) throw ctx.error("NOT_FOUND", "This post no longer exists.");
		const id = await ctx.db.unsafe.insert("Comment", {
			postId: args.postId,
			profileId: profile.id,
			text,
			createdAt: new Date().toISOString(),
		});
		return { id };
	},
});
