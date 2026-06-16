import { mutation, v } from "@pylonsync/functions";

// deleteConversation — remove a conversation AND all its messages in one
// transaction. The client could delete the Conversation row itself (it's
// owner-scoped), but that would orphan the messages; this cascades. Gated to the
// owner: it verifies the conversation belongs to the caller before deleting.
export default mutation<{ conversationId: string }, { ok: boolean; deleted: number }>({
  auth: "user",
  args: { conversationId: v.id("Conversation") },
  async handler(ctx, args) {
    const convo = (await ctx.db.get("Conversation", args.conversationId)) as
      | { userId: string }
      | null;
    if (!convo) return { ok: true, deleted: 0 };
    if (convo.userId !== ctx.auth.userId) {
      throw ctx.error("POLICY_DENIED", "You can only delete your own conversations.");
    }

    const messages = (await ctx.db.unsafe.list("Message")) as unknown as {
      id: string;
      conversationId: string;
    }[];
    let deleted = 0;
    for (const m of messages) {
      if (m.conversationId === args.conversationId) {
        await ctx.db.unsafe.delete("Message", m.id);
        deleted++;
      }
    }
    await ctx.db.unsafe.delete("Conversation", args.conversationId);
    return { ok: true, deleted };
  },
});
