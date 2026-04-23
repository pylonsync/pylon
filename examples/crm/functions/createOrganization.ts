import { mutation, v } from "@pylonsync/functions";

/** Create an org + owner OrgMember in one transaction, then caller switches
 *  session via /api/auth/select-org. */
export default mutation({
  args: { name: v.string(), slug: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const name = args.name.trim();
    if (name.length === 0 || name.length > 120)
      throw ctx.error("INVALID_NAME", "org name 1–120 chars");
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,49}$/.test(slug))
      throw ctx.error("INVALID_SLUG", "lowercase letters/numbers/dashes, 2–50");

    const now = new Date().toISOString();
    try {
      const orgId = await ctx.db.insert("Organization", {
        name, slug, createdBy: ctx.auth.userId, createdAt: now,
      });
      await ctx.db.insert("OrgMember", {
        userId: ctx.auth.userId,
        orgId,
        role: "owner",
        joinedAt: now,
      });
      return { orgId, slug };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" already taken`);
      throw e;
    }
  },
});
