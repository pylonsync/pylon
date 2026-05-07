import { mutation, v } from "@pylonsync/functions";

/**
 * Add a User to an Org with a role. Membership policy already gates
 * this to owners / admins; we additionally validate the role string
 * here (Pylon enforces enum at the schema level too, but a clearer
 * error message is nicer than a generic constraint failure).
 *
 * For a real app, replace this with an email-invite flow that emits
 * an Invite row + emails the user — but the wiring is the same: the
 * Membership row is what grants access.
 */
const ROLES = new Set(["owner", "admin", "member"]);

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
		const dup = (await ctx.db.query("Membership", {
			orgId: args.orgId,
			userId: args.userId,
		})) as any[];
		if (dup.length > 0) {
			throw ctx.error(
				"ALREADY_MEMBER",
				"user is already a member of this org",
			);
		}
		const id = await ctx.db.insert("Membership", {
			orgId: args.orgId,
			userId: args.userId,
			role: args.role,
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Membership", id);
	},
});
