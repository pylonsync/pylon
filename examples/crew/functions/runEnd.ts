import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    runId: v.string(),
    status: v.string(), // completed | failed | cancelled
    tokensIn: v.number(),
    tokensOut: v.number(),
    error: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    await ctx.db.update("Run", args.runId, {
      status: args.status,
      completedAt: new Date().toISOString(),
      tokensIn: args.tokensIn,
      tokensOut: args.tokensOut,
      error: args.error ?? null,
    });
    return { ok: true };
  },
});
