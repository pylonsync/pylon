import { query, v } from "@pylonsync/functions";

/**
 * Every Post by a single Profile, newest first. Used by the profile
 * page on each platform.
 */
export default query({
	args: { profileId: v.id("Profile") },
	async handler(ctx, args: { profileId: string }) {
		const rows = (await ctx.db.query("Post", {
			authorId: args.profileId,
		})) as any[];
		return rows.sort(
			(a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt),
		);
	},
});
