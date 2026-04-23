import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    name: v.string(),
    key: v.string(),
    description: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const name = args.name.trim();
    if (!name) throw ctx.error("INVALID_NAME", "team name required");
    const key = args.key.trim().toUpperCase();
    if (!/^[A-Z][A-Z0-9]{0,9}$/.test(key))
      throw ctx.error("INVALID_KEY", "team key: 1–10 uppercase letters/digits");

    const now = new Date().toISOString();
    try {
      const teamId = await ctx.db.insert("Team", {
        orgId: ctx.auth.tenantId,
        name,
        key,
        description: args.description?.trim() || null,
        issueSequence: 0,
        createdBy: ctx.auth.userId,
        createdAt: now,
      });
      await ctx.db.insert("TeamMember", {
        orgId: ctx.auth.tenantId,
        teamId,
        userId: ctx.auth.userId,
        joinedAt: now,
      });
      return { teamId, key };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("KEY_TAKEN", `team key "${key}" already in use`);
      throw e;
    }
  },
});
