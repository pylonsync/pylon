import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    name: v.string(),
    description: v.optional(v.string()),
    steps: v.array(
      v.object({ agentId: v.string(), instruction: v.string() }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name required");
    if (args.steps.length === 0)
      throw ctx.error("NO_STEPS", "pipeline needs at least one step");

    const now = new Date().toISOString();
    const pipelineId = await ctx.db.insert("Pipeline", {
      orgId: ctx.auth.tenantId,
      name,
      description: args.description?.trim() || null,
      createdBy: ctx.auth.userId,
      createdAt: now,
    });

    for (let i = 0; i < args.steps.length; i++) {
      const step = args.steps[i];
      // Confirm each referenced agent lives in this tenant before wiring it
      // in. Policies already gate the insert, but a clean error here beats
      // a cryptic constraint failure halfway through.
      const agent = await ctx.db.get("Agent", step.agentId);
      if (!agent || (agent as { orgId?: string }).orgId !== ctx.auth.tenantId) {
        throw ctx.error("INVALID_AGENT", `agent ${step.agentId} not in this org`);
      }
      await ctx.db.insert("PipelineStep", {
        orgId: ctx.auth.tenantId,
        pipelineId,
        position: i,
        agentId: step.agentId,
        instruction: step.instruction,
      });
    }

    return { pipelineId };
  },
});
