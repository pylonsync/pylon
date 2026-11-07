import { query, v } from "@pylonsync/functions";

/** List active projects after checking organization membership. */
export default query({
	args: { orgId: v.id("Org") },
	async handler(ctx, args: { orgId: string }) {
		await ctx.requireMember(args.orgId, { entity: "Membership" });
		const rows = (await ctx.db.query("Project", {
			orgId: args.orgId,
		})) as any[];
		return rows
			.filter((r) => !r.archived)
			.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
	},
});
