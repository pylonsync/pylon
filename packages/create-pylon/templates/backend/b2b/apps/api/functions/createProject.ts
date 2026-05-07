import { mutation, v } from "@pylonsync/functions";

/**
 * Create a Project inside an Org. Project policy already gates this
 * to org members; we additionally trim the name and stamp the caller
 * as creator.
 */
export default mutation({
	args: { orgId: v.id("Org"), name: v.string() },
	async handler(ctx, args: { orgId: string; name: string }) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const name = args.name.trim();
		if (!name) throw ctx.error("EMPTY_NAME", "project name cannot be empty");
		const id = await ctx.db.insert("Project", {
			orgId: args.orgId,
			name,
			createdBy: ctx.auth.userId,
			archived: false,
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Project", id);
	},
});
