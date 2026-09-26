import { query, v } from "@pylonsync/functions";
import { AGENT_NAME } from "../lib/assistant";

// Map the agent's current stream id to the conversation it is answering.
// Internal: called by the assistant's tools only (functions/assistant.ts).
//
// Returns null unless all of these hold:
//   - an AgentRun of the assistant carries this stream id and belongs to the caller
//   - its title is a conversation id (processTurn sets it)
//   - that conversation has a turn running right now
export default query({
  internal: true,
  auth: "admin",
  args: { streamId: v.string() },
  async handler(ctx, args) {
    if (args.streamId === "") return null;
    const runs = await ctx.db.query("AgentRun", { streamId: args.streamId, $limit: 2 });
    if (runs.length !== 1) return null;
    const run = runs[0];
    if (run.agent !== AGENT_NAME || run.userId !== ctx.auth.userId || run.status !== "running") return null;
    const conversation = await ctx.db.get("Conversation", String(run.title ?? ""));
    if (!conversation || conversation.turnState !== "running") return null;
    return { conversationId: String(conversation.id), contactId: String(conversation.contactId) };
  },
});
