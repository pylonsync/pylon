import { action, v } from "@pylonsync/functions";
import { AGENT_NAME } from "../lib/assistant";
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

      // The run belongs to the owner, so it shows up live in the dashboard.
      // `admin: true` because the assistant is declared auth: "admin".
      // The context tells the tools which conversation and turn they act on
      // (functions/assistant.ts); only server code can set it.
      const result = await ctx.agents.run(AGENT_NAME, {
        input: begin.input,
        ...(begin.runId ? { runId: begin.runId } : {}),
        as: { userId: owner.userId, admin: true },
        context: { conversationId: begin.conversationId, turnId: begin.turnId },
      });
      const chunks = chunkReply(result.text);
      if (chunks.length === 0) {
        await finish({ outcome: "failed", detail: "The model returned an empty reply", runId: result.runId });
        return { state: "failed" };
      }
      await finish({ outcome: "answered", runId: result.runId, chunks });
      return { state: "answered", bubbles: chunks.length };
    } catch (err) {
      const code = (err as { code?: unknown })?.code;
      const message = err instanceof Error ? err.message : String(err);
      const detail = typeof code === "string" ? `${code}: ${message}` : message;
      await finish({ outcome: "failed", detail: detail.slice(0, 500) });
      return { state: "failed" };
    }
  },
});
