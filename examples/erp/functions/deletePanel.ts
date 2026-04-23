import { mutation, v } from "@pylonsync/functions";

/**
 * Remove a panel from the org dashboard. Gate on tenantId so a caller
 * can't delete panels from another org by guessing the id.
 */
export default mutation({
  args: { panelId: v.id("DashboardPanel") },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const panel = await ctx.db.get("DashboardPanel", args.panelId);
    if (!panel || panel.orgId !== ctx.auth.tenantId) {
      throw ctx.error("NOT_FOUND", "panel does not exist in this org");
    }
    await ctx.db.delete("DashboardPanel", args.panelId);
    return { panelId: args.panelId };
  },
});
