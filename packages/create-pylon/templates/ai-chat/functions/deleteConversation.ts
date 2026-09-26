import { mutation, v } from "@pylonsync/functions";

// Delete a conversation and its messages in one transaction. Only the owner
// can delete; a missing or foreign id returns `deleted: 0` without saying
// which.
export default mutation({
  auth: "guest",
  args: { conversationId: v.string() },
  async handler(ctx, args) {
    const convo = (await ctx.db.unsafe.get("Conversation", args.conversationId)) as
      | { userId: string }
      | null;
    if (!convo || convo.userId !== ctx.auth.userId) return { ok: true, deleted: 0 };

    // Reads by conversation id, then checks each row's owner as well.
    const messages = (await ctx.db.unsafe.query("Message", {
      conversationId: args.conversationId,
    })) as { id: string; userId: string }[];
    let deleted = 0;
    for (const m of messages) {
      if (m.userId !== ctx.auth.userId) continue;
      await ctx.db.unsafe.delete("Message", m.id);
      deleted++;
    }
    await ctx.db.unsafe.delete("Conversation", args.conversationId);
    return { ok: true, deleted };
  },
});
