import { mutation, v } from "@pylonsync/functions";

/** Remove a member. Keep the founding owner in the organization. */
export default mutation({
	args: { orgId: v.id("Org"), userId: v.id("User") },
	async handler(ctx, args: { orgId: string; userId: string }) {
		await ctx.requireMember(args.orgId, { entity: "Membership", role: ["owner", "admin"] });
		const org = (await ctx.db.get("Org", args.orgId)) as any;
		if (!org) throw ctx.error("NOT_FOUND", "org not found");
		if (org.ownerId === args.userId) {
			throw ctx.error(
				"OWNER_CANT_LEAVE",
				"the org owner can't be removed; transfer ownership first",
			);
		}
		const memberships = (await ctx.db.query("Membership", {
			orgId: args.orgId,
			userId: args.userId,
		})) as any[];
		for (const m of memberships) {
			await ctx.db.delete("Membership", m.id);
		}
		return memberships[0] ?? null;
	},
});
