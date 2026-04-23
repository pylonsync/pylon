import { action, v } from "@pylonsync/functions";

// Stubbed "LLM". Real Anthropic swaps in here — the orchestrator doesn't
// care where the tokens come from, only that they arrive one at a time.
// Deterministic output keeps the example reproducible in demos and tests.
async function* fakeTokenStream(
  systemPrompt: string,
  input: string,
): AsyncGenerator<string, { tokensIn: number; tokensOut: number }, unknown> {
  const reply = draftReply(systemPrompt, input);
  const tokens = reply.split(/(\s+)/).filter((t) => t.length > 0);
  for (const tok of tokens) {
    await sleep(30 + Math.random() * 40);
    yield tok;
  }
  return {
    tokensIn: roughTokenCount(systemPrompt) + roughTokenCount(input),
    tokensOut: roughTokenCount(reply),
  };
}

function draftReply(systemPrompt: string, input: string): string {
  const role = (systemPrompt.match(/You are an? ([^.\n]+)/i)?.[1] || "assistant").trim();
  const subject = input.trim().replace(/\s+/g, " ").slice(0, 140);
  const bullets = [
    `Thinking through "${subject}" as your ${role}.`,
    `Key angle: treat this as a concrete, shippable piece of work — not theory.`,
    `Three things that matter most:`,
    `  1. Scope it tight. Pick the minimum viable version that answers the real question.`,
    `  2. Name the risks up front. One sentence each, no hedging.`,
    `  3. Draft the output, then cut a third of it.`,
    `Where I'd push back: assumptions hiding in the phrasing of the ask.`,
    `Next step: turn this into a concrete deliverable in the agent that follows.`,
  ];
  return bullets.join("\n");
}

function roughTokenCount(s: string): number {
  return Math.ceil(s.length / 4);
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

function renderInstruction(
  template: string,
  vars: { input: string; previous: string },
): string {
  return template
    .replaceAll("{{input}}", vars.input)
    .replaceAll("{{previous}}", vars.previous);
}

// Client resolves the pipeline/agent graph against its local sync replica
// and passes the flat step list here. This avoids `ctx.runQuery` inside
// the action, which drops `tenant_id` on nested calls (framework
// limitation). The server still policy-gates every write the action
// emits, so this doesn't widen the trust boundary — the client just
// loses the ability to request a run against rows it couldn't otherwise
// see.
export default action({
  args: {
    pipelineId: v.optional(v.string()),
    agentId: v.optional(v.string()),
    input: v.string(),
    title: v.optional(v.string()),
    steps: v.array(
      v.object({
        agentId: v.string(),
        systemPrompt: v.string(),
        instruction: v.string(),
      }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    if (args.steps.length === 0) throw ctx.error("EMPTY_PLAN", "nothing to run");

    const title =
      (args.title && args.title.trim()) ||
      args.input.trim().slice(0, 60) ||
      "Untitled run";

    const { runId } = await ctx.runMutation<{ runId: string }>("runStart", {
      pipelineId: args.pipelineId,
      agentId: args.agentId,
      title,
      input: args.input,
    });

    let previous = "";
    let totalIn = 0;
    let totalOut = 0;

    try {
      for (let i = 0; i < args.steps.length; i++) {
        const step = args.steps[i];
        const stepInput = renderInstruction(step.instruction, {
          input: args.input,
          previous,
        });

        const { stepId, messageId } = await ctx.runMutation<{
          stepId: string;
          messageId: string;
        }>("runStepStart", {
          runId,
          stepNumber: i + 1,
          agentId: step.agentId,
          input: stepInput,
        });

        let accumulated = "";
        const gen = fakeTokenStream(step.systemPrompt, stepInput);
        let flushAt = 0;
        let final: { tokensIn: number; tokensOut: number } = { tokensIn: 0, tokensOut: 0 };
        while (true) {
          const next = await gen.next();
          if (next.done) {
            final = next.value;
            break;
          }
          accumulated += next.value;
          const now = Date.now();
          if (now - flushAt > 80) {
            flushAt = now;
            await ctx.runMutation("runStepAppend", {
              stepId,
              messageId,
              output: accumulated,
            });
          }
        }

        await ctx.runMutation("runStepEnd", {
          stepId,
          output: accumulated,
          tokensIn: final.tokensIn,
          tokensOut: final.tokensOut,
          status: "completed",
        });
        totalIn += final.tokensIn;
        totalOut += final.tokensOut;
        previous = accumulated;
      }

      await ctx.runMutation("runEnd", {
        runId,
        status: "completed",
        tokensIn: totalIn,
        tokensOut: totalOut,
      });
      return { runId, status: "completed" };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      await ctx.runMutation("runEnd", {
        runId,
        status: "failed",
        tokensIn: totalIn,
        tokensOut: totalOut,
        error: msg,
      });
      throw e;
    }
  },
});
