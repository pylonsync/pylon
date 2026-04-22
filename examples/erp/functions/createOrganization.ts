import { mutation, v } from "@statecraft/functions";

/**
 * Create a new organization + the caller's owner membership in a single
 * transaction. The caller must then hit /api/auth/select-org to switch
 * their session to it; this function returns the orgId so the client can
 * do that right after.
 */
export default mutation({
  args: {
    name: v.string(),
    slug: v.string(),
    billingEmail: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const name = args.name.trim();
    if (name.length === 0 || name.length > 120) {
      throw ctx.error("INVALID_NAME", "org name must be 1–120 chars");
    }
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,49}$/.test(slug)) {
      throw ctx.error(
        "INVALID_SLUG",
        "slug: lowercase letters/numbers/dashes, 2–50 chars",
      );
    }

    const now = new Date().toISOString();

    try {
      const orgId = await ctx.db.insert("Organization", {
        name,
        slug,
        billingEmail: args.billingEmail ?? null,
        createdBy: ctx.auth.userId,
        createdAt: now,
      });

      // The creator is automatically the owner.
      await ctx.db.insert("OrgMember", {
        userId: ctx.auth.userId,
        orgId,
        role: "owner",
        invitedBy: null,
        joinedAt: now,
      });

      return { orgId, slug };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg)) {
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" already taken`);
      }
      throw e;
    }
  },
});
