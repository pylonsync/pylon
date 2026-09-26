import { mutation, v } from "@pylonsync/functions";

// Internal: store the final text of a reply, called by `respond` when the
// provider call ends or fails. Ownership is checked against the ReplyLog row
// `_beginTurn` wrote for the same caller, so the reply still lands after a
// guest signs in and its messages move to the account.
//
// A reply the user stopped keeps the text they saw, but only when that text
// is a prefix of what the model produced. Otherwise the stored text is the
// model's own output cut to the same length, so a client cannot write words
// into the assistant's turn.
export default mutation({
  internal: true,
  auth: "guest",
  args: {
    messageId: v.string(),
    content: v.string(),
    status: v.string(),
    errorCode: v.optional(v.string()),
  },
  async handler(ctx, args) {
    // ReplyLog and Message are server-only for writes; the log row proves
    // this caller started the generation.
    const [log] = (await ctx.db.unsafe.query("ReplyLog", { messageId: args.messageId, $limit: 1 })) as {
      id: string;
      userId: string;
    }[];
    if (!log || log.userId !== ctx.auth.userId) return { ok: false };
    await ctx.db.unsafe.update("ReplyLog", log.id, { finishedAt: new Date().toISOString() });

    const msg = (await ctx.db.unsafe.get("Message", args.messageId)) as
      | { status: string; content: string; conversationId: string }
      | null;
    if (!msg) return { ok: true };

    if (msg.status === "stopped") {
      const shown = msg.content;
      const content = args.content.startsWith(shown) ? shown : args.content.slice(0, shown.length);
      if (content !== shown) await ctx.db.unsafe.update("Message", args.messageId, { content });
      return { ok: true };
    }
    if (msg.status !== "streaming") return { ok: true };

    const status = args.status === "error" ? "error" : "done";
    await ctx.db.unsafe.update("Message", args.messageId, {
      content: args.content,
      status,
      errorCode: status === "error" ? (args.errorCode ?? "LLM_FAILED") : null,
    });
    await ctx.db.unsafe.update("Conversation", msg.conversationId, {
      updatedAt: new Date().toISOString(),
    });
    return { ok: true };
  },
});
