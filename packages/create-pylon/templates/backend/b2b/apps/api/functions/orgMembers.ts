import { query, v } from "@pylonsync/functions";

/**
 * List every member of an Org. Membership policy gates this to
 * org-admins + owners (members see only their own row), so non-admin
 * callers get a single-element array; admins get everyone.
 */
export default query({
	args: { orgId: v.id("Org") },
	async handler(ctx, args: { orgId: string }) {
		return await ctx.db.query("Membership", { orgId: args.orgId });
	},
});
