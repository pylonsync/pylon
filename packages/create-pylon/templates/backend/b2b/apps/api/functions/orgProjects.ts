import { query, v } from "@pylonsync/functions";

/**
 * Every Project in an Org. Project policy enforces tenant scope, so
 * even if the caller passes a foreign `orgId` they're not a member of,
 * the query returns nothing.
 */
export default query({
	args: { orgId: v.id("Org") },
	async handler(ctx, args: { orgId: string }) {
		const rows = (await ctx.db.query("Project", {
			orgId: args.orgId,
		})) as any[];
		return rows
			.filter((r) => !r.archived)
			.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
	},
});
