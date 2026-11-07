import { mutation, v } from "@pylonsync/functions";

/** Archive a project after checking organization administration access. */
export default mutation({
	args: { id: v.id("Project") },
	async handler(ctx, args: { id: string }) {
		const project = await ctx.db.get("Project", args.id);
		if (!project) throw ctx.error("NOT_FOUND", "project not found");
		await ctx.requireMember(project.orgId as string, { entity: "Membership", role: ["owner", "admin"] });
		await ctx.db.update("Project", args.id, { archived: true });
		return await ctx.db.get("Project", args.id);
	},
});
