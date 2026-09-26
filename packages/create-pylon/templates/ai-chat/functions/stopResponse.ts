import { mutation, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";

// Stop the reply streaming in a conversation. The client sends the text it
// has shown so far; the message keeps it with status "stopped", and
// `_finishTurn` checks it against the model's output when the call ends.
//
// The provider call itself runs to completion on the server: `ctx.llm.stream`
// has no mid-generation cancel, so tokens after Stop are generated and billed
// but not stored. `_beginTurn` refuses a new turn until that call ends.
export default mutation({
  auth: "guest",
  args: { conversationId: v.string(), content: v.string() },
  async handler(ctx, args) {
    // Message updates are denied to clients by policy; only the caller's own
    // streaming reply in this conversation is touched.
    const rows = (await ctx.db.unsafe.query("Message", {
      conversationId: args.conversationId,
      role: "assistant",
      status: "streaming",
    })) as { id: string; userId: string }[];
    const msg = rows.find((m) => m.userId === ctx.auth.userId);
    if (!msg) return { ok: true, stopped: false };
    await ctx.db.unsafe.update("Message", msg.id, {
      status: "stopped",
      content: args.content.slice(0, siteConfig.chat.maxOutputTokens * 8),
    });
    return { ok: true, stopped: true };
  },
});
