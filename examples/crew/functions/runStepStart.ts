import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    runId: v.string(),
    stepNumber: v.number(),
    agentId: v.string(),
    input: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const run = await ctx.db.get("Run", args.runId);
    if (!run || (run as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "run not found");

    const now = new Date().toISOString();
    const stepId = await ctx.db.insert("RunStep", {
      orgId: ctx.auth.tenantId,
      runId: args.runId,
      stepNumber: args.stepNumber,
      agentId: args.agentId,
      input: args.input,
      output: "",
      status: "running",
      tokensIn: null,
      tokensOut: null,
      startedAt: now,
      completedAt: null,
      error: null,
    });

    // The user-visible "prompt" message that kicked this step off — rendered
    // in the transcript as the input bubble for this step.
    await ctx.db.insert("Message", {
      orgId: ctx.auth.tenantId,
      runId: args.runId,
      runStepId: stepId,
      role: "user",
      content: args.input,
      createdAt: now,
    });

    // Pre-seed the assistant response as an empty message. `runStepAppend`
    // will update this same row's content as tokens arrive — each update
    // is a change event the sync engine broadcasts, so every watcher sees
    // the response build in real time without client-side message-merge
    // logic.
    const messageId = await ctx.db.insert("Message", {
      orgId: ctx.auth.tenantId,
      runId: args.runId,
      runStepId: stepId,
      role: "assistant",
      content: "",
      createdAt: new Date().toISOString(),
    });

    return { stepId, messageId };
  },
});
