import { mutation, v } from "@pylonsync/functions";

const ROLES = new Set(["owner", "admin", "member"]);

/**
 * Promote/demote a member. Only one row per (orgId, userId) so we
 * find by composite then update. Setting role=owner does NOT change
 * Org.ownerId — those are separate concepts (the founder vs current
 * org-leadership members); this scaffold leaves transfer-of-ownership
 * as a separate flow.
 */
export default mutation({
	args: {
		orgId: v.id("Org"),
		userId: v.id("User"),
		role: v.string(),
	},
	async handler(
		ctx,
		args: { orgId: string; userId: string; role: string },
	) {
		if (!ROLES.has(args.role)) {
			throw ctx.error(
				"INVALID_ROLE",
				`role must be one of ${[...ROLES].join(", ")}`,
			);
		}
		const memberships = (await ctx.db.query("Membership", {
			orgId: args.orgId,
			userId: args.userId,
		})) as any[];
		if (memberships.length === 0) {
			throw ctx.error("NOT_A_MEMBER", "user is not a member of this org");
		}
		await ctx.db.update("Membership", memberships[0].id, { role: args.role });
		return await ctx.db.get("Membership", memberships[0].id);
	},
});
