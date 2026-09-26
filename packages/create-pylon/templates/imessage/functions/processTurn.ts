import { action, v } from "@pylonsync/functions";
import { runAgentAsOwner } from "../lib/agent-call";
import { chunkReply } from "../lib/chunk";
import { providerStatus } from "../lib/provider";

type Begin =
  | { state: "idle" | "busy" }
  | { state: "started"; turnId: string; conversationId: string; runId: string | null; input: string };

// One agent turn for a conversation: claim the waiting texts, run the
// `assistant` agent as the owner, split the reply into bubbles, and hand them
// to finishTurn for delivery. Scheduled by ingestInbound; internal.
//
// Every failure is recorded on the texts and the conversation rather than
// thrown, so the job queue does not retry a turn that already reached the
// model.
export default action({
  internal: true,
  auth: "admin",
  timeout: 300,
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const begin = await ctx.runMutation<Begin>("beginTurn", { conversationId: args.conversationId });
    if (begin.state !== "started") return { state: begin.state };

    const finish = (fields: Record<string, unknown>) =>
      ctx.runMutation("finishTurn", {
        conversationId: begin.conversationId,
        turnId: begin.turnId,
        ...fields,
      });

    try {
      if (!providerStatus(ctx.env).configured) {
        await finish({
          outcome: "awaiting_setup",
          detail: "No model provider key is set. Add ANTHROPIC_API_KEY or OPENAI_API_KEY, then answer from the contact details.",
        });
        return { state: "awaiting_setup" };
      }
      const owner = await ctx.runQuery<{ userId: string } | null>("ownerAccount", {});
      if (!owner) {
        await finish({
          outcome: "awaiting_setup",
          detail: "No owner account yet. Sign in at /login with the email in PYLON_ADMIN_EMAILS.",
        });
        return { state: "awaiting_setup" };
      }

      const outcome = await runAgentAsOwner(ctx.env, owner.userId, {
        input: begin.input,
        ...(begin.runId ? { runId: begin.runId } : {}),
        // The run's title carries the conversation id. The agent's tools read
        // it back (functions/resolveTurn.ts) to know which contact they act on.
        title: begin.conversationId,
      });
      if (!outcome.ok) {
        await finish({ outcome: "failed", detail: `${outcome.code}: ${outcome.message}`.slice(0, 500) });
        return { state: "failed" };
      }
      const chunks = chunkReply(outcome.result.text);
      if (chunks.length === 0) {
        await finish({ outcome: "failed", detail: "The model returned an empty reply", runId: outcome.result.runId });
        return { state: "failed" };
      }
      await finish({ outcome: "answered", runId: outcome.result.runId, chunks });
      return { state: "answered", bubbles: chunks.length };
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      await finish({ outcome: "failed", detail: message.slice(0, 500) });
      return { state: "failed" };
    }
  },
});
