import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { name: v.string(), slug: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name required");
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,49}$/.test(slug))
      throw ctx.error("INVALID_SLUG", "lowercase letters/numbers/dashes, 2–50");

    const now = new Date().toISOString();
    try {
      const orgId = await ctx.db.insert("Organization", {
        name, slug, createdBy: ctx.auth.userId, createdAt: now,
      });
      await ctx.db.insert("OrgMember", {
        userId: ctx.auth.userId, orgId, role: "owner", joinedAt: now,
      });
      // Default team — every org starts with one so users can file issues
      // immediately without a separate setup step.
      const teamId = await ctx.db.insert("Team", {
        orgId,
        name: "Engineering",
        key: "ENG",
        description: null,
        issueSequence: 0,
        createdBy: ctx.auth.userId,
        createdAt: now,
      });
      await ctx.db.insert("TeamMember", {
        orgId, teamId, userId: ctx.auth.userId, joinedAt: now,
      });
      return { orgId, teamId };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" already taken`);
      throw e;
    }
  },
});
