import { mutation, v } from "@pylonsync/functions";

/** Move a deal to a new pipeline stage. Stamps wonAt/lostAt when landing
 *  on those states, and writes an Activity row. */
export default mutation({
  args: {
    dealId: v.id("Deal"),
    stage: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const valid = ["lead", "qualified", "proposal", "negotiation", "won", "lost"];
    if (!valid.includes(args.stage))
      throw ctx.error("INVALID_STAGE", `stage ∈ ${valid.join(", ")}`);

    const deal = await ctx.db.get("Deal", args.dealId);
    if (!deal || deal.orgId !== ctx.auth.tenantId)
      throw ctx.error("DEAL_NOT_FOUND", "deal not in this org");
    if (deal.stage === args.stage) return { dealId: args.dealId, changed: false };

    const now = new Date().toISOString();
    const patch: Record<string, unknown> = {
      stage: args.stage,
      updatedAt: now,
    };
    if (args.stage === "won") patch.wonAt = now;
    if (args.stage === "lost") patch.lostAt = now;

    await ctx.db.update("Deal", args.dealId, patch);
    await ctx.db.insert("Activity", {
      orgId: ctx.auth.tenantId,
      targetType: "Deal",
      targetId: args.dealId,
      kind: "stage_changed",
      metaJson: JSON.stringify({ from: deal.stage, to: args.stage }),
      actorId: ctx.auth.userId,
      createdAt: now,
    });
    return { dealId: args.dealId, changed: true };
  },
});
