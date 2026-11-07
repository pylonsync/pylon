import { mutation, v } from "@pylonsync/functions";

/** Add a member after checking the caller is an organization administrator. */
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
		await ctx.requireMember(args.orgId, { entity: "Membership", role: ["owner", "admin"] });
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
