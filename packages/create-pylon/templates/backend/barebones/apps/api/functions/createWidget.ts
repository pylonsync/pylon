import { mutation, v } from "@pylonsync/functions";

/**
 * Insert a new Widget. Initializes count to 0 and stamps the time.
 * Returns the inserted row so the client can render it without a
 * follow-up read.
 */
export default mutation({
	args: { name: v.string() },
	async handler(ctx, args: { name: string }) {
		const trimmed = args.name.trim();
		if (!trimmed) {
			throw ctx.error("EMPTY_NAME", "name cannot be empty");
		}
		const id = await ctx.db.insert("Widget", {
			name: trimmed,
			count: 0,
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Widget", id);
	},
});
