import { mutation, v } from "@pylonsync/functions";

/**
 * Create a new Org with the caller as owner. Atomically writes the
 * Org row + the founding Membership; both must succeed or neither
 * does (Pylon mutations run in a single transaction).
 */
export default mutation({
	args: { slug: v.string(), name: v.string() },
	async handler(ctx, args: { slug: string; name: string }) {
		if (!ctx.auth.userId) {
			throw ctx.error("UNAUTHENTICATED", "log in first");
		}
		const slug = args.slug.trim().toLowerCase();
		if (!/^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$/.test(slug)) {
			throw ctx.error(
				"INVALID_SLUG",
				"slug must be 3–32 chars, alphanumeric + dashes, no leading/trailing dash",
			);
		}
		const name = args.name.trim();
		if (!name) {
			throw ctx.error("EMPTY_NAME", "org name cannot be empty");
		}
		const dup = (await ctx.db.query("Org", { slug })) as any[];
		if (dup.length > 0) {
			throw ctx.error("SLUG_TAKEN", `org slug "${slug}" is taken`);
		}
		const orgId = await ctx.db.insert("Org", {
			slug,
			name,
			ownerId: ctx.auth.userId,
			createdAt: new Date().toISOString(),
		});
		await ctx.db.insert("Membership", {
			orgId,
			userId: ctx.auth.userId,
			role: "owner",
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Org", orgId);
	},
});
