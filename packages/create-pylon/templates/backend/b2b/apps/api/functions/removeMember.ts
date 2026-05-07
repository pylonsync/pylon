import { mutation, v } from "@pylonsync/functions";

/**
 * Remove a User from an Org. Refuses to remove the org's owner
 * (transfer ownership first via setMemberRole + a manual Org.ownerId
 * update — left as an exercise; this scaffold stops at the safety
 * check).
 */
export default mutation({
	args: { orgId: v.id("Org"), userId: v.id("User") },
	async handler(ctx, args: { orgId: string; userId: string }) {
		const org = (await ctx.db.get("Org", args.orgId)) as any;
		if (!org) throw ctx.error("NOT_FOUND", "org not found");
		if (org.ownerId === args.userId) {
			throw ctx.error(
				"OWNER_CANT_LEAVE",
				"the org owner can't be removed; transfer ownership first",
			);
		}
		const memberships = (await ctx.db.query("Membership", {
			orgId: args.orgId,
			userId: args.userId,
		})) as any[];
		for (const m of memberships) {
			await ctx.db.delete("Membership", m.id);
		}
		return memberships[0] ?? null;
	},
});
