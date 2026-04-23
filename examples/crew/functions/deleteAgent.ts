import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { agentId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");

    const agent = await ctx.db.get("Agent", args.agentId);
    if (!agent) return { deleted: false };
    if ((agent as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("FORBIDDEN", "not in this org");

    // Cascade manually: pipeline steps referencing this agent would break
    // future runs. Leave existing runs alone (historical), just prevent new
    // invocations by dropping the pipeline-step wiring.
    const steps = await ctx.db.query("PipelineStep", { agentId: args.agentId });
    for (const step of steps as { id: string }[]) {
      await ctx.db.delete("PipelineStep", step.id);
    }

    await ctx.db.delete("Agent", args.agentId);
    return { deleted: true };
  },
});
