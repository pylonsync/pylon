import { query } from "@pylonsync/functions";

/**
 * Resolve the caller's own Profile (or null if they haven't created
 * one yet). Used by the client to decide whether to show the
 * profile-setup screen on first launch.
 */
export default query({
	args: {},
	async handler(ctx) {
		if (!ctx.auth.userId) return null;
		const rows = (await ctx.db.query("Profile", {
			userId: ctx.auth.userId,
		})) as any[];
		return rows[0] ?? null;
	},
});
