import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    stepId: v.string(),
    messageId: v.string(),
    output: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    // Policies already gate these writes by orgId == tenantId; skipping an
    // extra read here keeps the per-chunk hot path lean. The action loop
    // can fire dozens of these per step.
    await ctx.db.update("Message", args.messageId, { content: args.output });
    await ctx.db.update("RunStep", args.stepId, { output: args.output });
    return { ok: true };
  },
});
