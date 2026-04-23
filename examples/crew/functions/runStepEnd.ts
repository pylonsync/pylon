import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    stepId: v.string(),
    output: v.string(),
    tokensIn: v.number(),
    tokensOut: v.number(),
    status: v.string(), // completed | failed | cancelled
    error: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    await ctx.db.update("RunStep", args.stepId, {
      output: args.output,
      status: args.status,
      tokensIn: args.tokensIn,
      tokensOut: args.tokensOut,
      completedAt: new Date().toISOString(),
      error: args.error ?? null,
    });
    return { ok: true };
  },
});
