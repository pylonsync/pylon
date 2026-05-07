import { query } from "@pylonsync/functions";

/**
 * Public feed — every Post, newest first. Joins author Profile + the
 * caller's like state into the response so the client can render a
 * row without N+1 follow-up queries.
 *
 * For a real consumer app, paginate this (limit + cursor by createdAt)
 * — the scaffold returns at most 100 rows, which is plenty for a
 * Hello-World feed but unsustainable past a few thousand posts.
 */
export default query({
	args: {},
	async handler(ctx) {
		const posts = (await ctx.db.query("Post", {})) as any[];
		const sorted = [...posts]
			.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
			.slice(0, 100);

		const callerProfile = ctx.auth.userId
			? ((await ctx.db.query("Profile", {
					userId: ctx.auth.userId,
				})) as any[])[0]
			: null;

		const out: Array<{
			id: string;
			body: string;
			createdAt: string;
			author: { id: string; handle: string; displayName: string } | null;
			likeCount: number;
			likedByMe: boolean;
		}> = [];
		for (const p of sorted) {
			const author = (await ctx.db.get("Profile", p.authorId)) as any;
			const likes = (await ctx.db.query("Like", { postId: p.id })) as any[];
			const likedByMe = callerProfile
				? likes.some((l) => l.profileId === callerProfile.id)
				: false;
			out.push({
				id: p.id,
				body: p.body,
				createdAt: p.createdAt,
				author: author
					? {
							id: author.id,
							handle: author.handle,
							displayName: author.displayName,
						}
					: null,
				likeCount: likes.length,
				likedByMe,
			});
		}
		return out;
	},
});
