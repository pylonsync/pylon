import { mutation, v } from "@pylonsync/functions";

// Bookkeeping mutations the `startRun` action calls to record progress.
// Splitting these out of the action lets each step become its own write
// transaction — which means each Message insert becomes a change event
// the sync engine broadcasts to every watcher in real time. A single
// transactional mutation would batch all the inserts and the UI would
// render the whole run at once.
export default mutation({
  args: {
    pipelineId: v.optional(v.string()),
    agentId: v.optional(v.string()),
    title: v.string(),
    input: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    if (!args.pipelineId && !args.agentId)
      throw ctx.error("INVALID_TARGET", "must specify pipelineId or agentId");

    const now = new Date().toISOString();
    const runId = await ctx.db.insert("Run", {
      orgId: ctx.auth.tenantId,
      pipelineId: args.pipelineId ?? null,
      agentId: args.agentId ?? null,
      title: args.title,
      input: args.input,
      status: "running",
      startedBy: ctx.auth.userId,
      createdAt: now,
      startedAt: now,
      completedAt: null,
      error: null,
      tokensIn: null,
      tokensOut: null,
    });
    return { runId };
  },
});
