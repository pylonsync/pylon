import { mutation, v } from "@pylonsync/functions";

/**
 * Soft-delete: set archived = true. Project policy gates updates to
 * org owner/admin only, so a regular member can't archive a project
 * even if they created it. Restore by writing archived = false (this
 * scaffold doesn't ship a restore action — left as an exercise).
 */
export default mutation({
	args: { id: v.id("Project") },
	async handler(ctx, args: { id: string }) {
		await ctx.db.update("Project", args.id, { archived: true });
		return await ctx.db.get("Project", args.id);
	},
});
