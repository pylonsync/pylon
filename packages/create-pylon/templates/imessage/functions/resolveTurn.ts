import { query, v } from "@pylonsync/functions";

// The contact a tool call acts on. Internal: called by the assistant's tools
// only (functions/assistant.ts), with the conversation and turn that
// processTurn put in the agent run's context.
//
// Returns null unless that conversation exists and that exact turn is still
// running, so a tool cannot act for a finished turn (for example when the
// owner continues the run from the dashboard later).
export default query({
  internal: true,
  auth: "admin",
  args: { conversationId: v.string(), turnId: v.string() },
  async handler(ctx, args) {
    if (args.conversationId === "" || args.turnId === "") return null;
    const conversation = await ctx.db.get("Conversation", args.conversationId);
    if (!conversation || conversation.turnState !== "running" || conversation.turnId !== args.turnId) return null;
    return { conversationId: String(conversation.id), contactId: String(conversation.contactId) };
  },
});
