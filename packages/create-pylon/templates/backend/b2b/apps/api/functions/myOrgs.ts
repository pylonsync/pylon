import { query } from "@pylonsync/functions";

/**
 * List every Org the caller belongs to. Joins Membership against Org
 * so the response carries everything the org switcher needs to render
 * a row (name, slug, the caller's role).
 */
export default query({
	args: {},
	async handler(ctx) {
		if (!ctx.auth.userId) return [];
		const memberships = await ctx.db.query("Membership", {
			userId: ctx.auth.userId,
		});
		const out: Array<{
			id: string;
			slug: string;
			name: string;
			role: string;
			createdAt: string;
		}> = [];
		for (const m of memberships as any[]) {
			const org = (await ctx.db.get("Org", m.orgId)) as any;
			if (!org) continue;
			out.push({
				id: org.id,
				slug: org.slug,
				name: org.name,
				role: m.role,
				createdAt: org.createdAt,
			});
		}
		return out.sort((a, b) =>
			Date.parse(b.createdAt) - Date.parse(a.createdAt),
		);
	},
});
