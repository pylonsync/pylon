import { query, v } from "@pylonsync/functions";

/** Administrators can list all members. Other members can read their own row. */
export default query({
	args: { orgId: v.id("Org") },
	async handler(ctx, args: { orgId: string }) {
		const member = await ctx.requireMember(args.orgId, { entity: "Membership" });
		const filter = member.role === "owner" || member.role === "admin"
			? { orgId: args.orgId }
			: { orgId: args.orgId, userId: ctx.auth.userId };
		return await ctx.db.query("Membership", filter);
	},
});
