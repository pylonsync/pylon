import { query, v } from "@pylonsync/functions";

// QueryCtx doesn't expose `ctx.error`; throw tagged Errors so the runtime
// surfaces the code instead of redacting to "Internal handler error".
function fail(code: string, message: string): Error {
  const err = new Error(message);
  (err as any).code = code;
  return err;
}

// Lookup helper the startRun action uses to resolve the agent+instruction
// for each step before entering the streaming loop. Kept separate from the
// action so the action's db access is read-only + explicit.
export default query({
  args: {
    pipelineId: v.optional(v.string()),
    agentId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw fail("NO_ACTIVE_ORG", "select an org first");

    if (args.agentId) {
      const agent = await ctx.db.get("Agent", args.agentId);
      if (!agent || (agent as { orgId?: string }).orgId !== ctx.auth.tenantId)
        throw fail("NOT_FOUND", "agent not found");
      return {
        steps: [
          {
            agentId: args.agentId,
            agentName: (agent as { name: string }).name,
            systemPrompt: (agent as { systemPrompt: string }).systemPrompt,
            instruction: "{{input}}",
          },
        ],
      };
    }

    if (args.pipelineId) {
      const pipeline = await ctx.db.get("Pipeline", args.pipelineId);
      if (!pipeline || (pipeline as { orgId?: string }).orgId !== ctx.auth.tenantId)
        throw fail("NOT_FOUND", "pipeline not found");

      const steps = await ctx.db.query("PipelineStep", {
        pipelineId: args.pipelineId,
      });
      steps.sort((a, b) => (a.position as number) - (b.position as number));

      const resolved: Array<{
        agentId: string;
        agentName: string;
        systemPrompt: string;
        instruction: string;
      }> = [];
      for (const s of steps) {
        const agent = await ctx.db.get("Agent", s.agentId as string);
        if (!agent) continue;
        resolved.push({
          agentId: s.agentId as string,
          agentName: (agent as { name: string }).name,
          systemPrompt: (agent as { systemPrompt: string }).systemPrompt,
          instruction: s.instruction as string,
        });
      }
      return { steps: resolved };
    }

    throw fail("INVALID_TARGET", "must specify pipelineId or agentId");
  },
});
