import { mutation, v } from "@pylonsync/functions";

/**
 * Append a message. authorId comes from `ctx.auth.userId`; the
 * authorName the client passes is denormalized into the row so we
 * don't have to join Profile (or any equivalent table) on every
 * read. For real apps, swap this for a Profile join — denormalized
 * names go stale when the user renames themselves.
 */
export default mutation({
	args: {
		roomId: v.id("Room"),
		body: v.string(),
		authorName: v.string(),
	},
	async handler(
		ctx,
		args: { roomId: string; body: string; authorName: string },
	) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const body = args.body.trim();
		if (!body) throw ctx.error("EMPTY_BODY", "message body cannot be empty");
		if (body.length > 2000) {
			throw ctx.error("TOO_LONG", "message capped at 2000 chars");
		}
		const id = await ctx.db.insert("Message", {
			roomId: args.roomId,
			authorId: ctx.auth.userId,
			authorName: args.authorName.trim() || "anonymous",
			body,
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Message", id);
	},
});
