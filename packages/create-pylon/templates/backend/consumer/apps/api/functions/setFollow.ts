import { mutation, v } from "@pylonsync/functions";
import { requireProfile } from "../lib/social";

/**
 * Follow or unfollow a profile as the caller. Idempotent, like setLike.
 */
export default mutation({
	args: { profileId: v.id("Profile"), following: v.boolean() },
	async handler(ctx, args: { profileId: string; following: boolean }) {
		const me = await requireProfile(ctx, ctx.auth.userId);
		if (me.id === args.profileId) {
			throw ctx.error("SELF_FOLLOW", "You cannot follow yourself.");
		}
		const target = await ctx.db.get("Profile", args.profileId);
		if (!target) throw ctx.error("NOT_FOUND", "This profile no longer exists.");

		await ctx.db.advisoryLock(`follow:${me.id}:${args.profileId}`);
		const existing = await ctx.db.query("Follow", {
			followerId: me.id,
			followingId: args.profileId,
		});
		if (args.following && existing.length === 0) {
			await ctx.db.unsafe.insert("Follow", {
				followerId: me.id,
				followingId: args.profileId,
				createdAt: new Date().toISOString(),
			});
		}
		if (!args.following) {
			for (const row of existing) {
				await ctx.db.unsafe.delete("Follow", String(row.id));
			}
		}
		return { following: args.following };
	},
});
