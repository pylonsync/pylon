import { mutation, v } from "@pylonsync/functions";

/**
 * Create a chat room. Slug must be unique (lowercase + dashes).
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
				"slug must be 3–32 chars: lowercase letters, digits, dashes",
			);
		}
		const name = args.name.trim();
		if (!name) throw ctx.error("EMPTY_NAME", "room name cannot be empty");
		const dup = (await ctx.db.query("Room", { slug })) as any[];
		if (dup.length > 0) {
			throw ctx.error("SLUG_TAKEN", `room slug "${slug}" is taken`);
		}
		const id = await ctx.db.insert("Room", {
			slug,
			name,
			createdAt: new Date().toISOString(),
		});
		return await ctx.db.get("Room", id);
	},
});
