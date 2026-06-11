import { mutation, v } from "@pylonsync/functions";

/**
 * Create a label for the given team. Labels are team-scoped so each team can
 * have its own taxonomy (e.g. "bug", "feature", "debt") without polluting
 * other teams' issue views.
 */
export default mutation({
  auth: "guest",
  args: {
    teamId: v.id("Team"),
    name: v.string(),
    color: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const name = (args.name as string).trim();
    if (!name) throw ctx.error("INVALID_NAME", "label name required");
    const color = (args.color as string).trim();
    if (!/^#[0-9a-fA-F]{6}$/.test(color))
      throw ctx.error("INVALID_COLOR", "color must be a #rrggbb hex string");

    const team = await ctx.db.get("Team", args.teamId as string);
    if (!team || team.orgId !== ctx.auth.tenantId)
      throw ctx.error("TEAM_NOT_FOUND", "team not in this org");

    const now = new Date().toISOString();
    try {
      const labelId = await ctx.db.insert("Label", {
        orgId: ctx.auth.tenantId,
        teamId: args.teamId,
        name,
        color,
        createdAt: now,
      });
      return { labelId, name, color };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("LABEL_EXISTS", `label "${name}" already exists on this team`);
      throw e;
    }
  },
});
