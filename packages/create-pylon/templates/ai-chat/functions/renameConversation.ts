import { mutation, v } from "@pylonsync/functions";
import { cleanTitle } from "../lib/chat";

// Rename one of the caller's conversations. Titles are trimmed to 80
// characters.
export default mutation({
  auth: "guest",
  args: { conversationId: v.string(), title: v.string() },
  async handler(ctx, args) {
    const title = cleanTitle(args.title.slice(0, 400));
    if (!title) throw ctx.error("INVALID_ARGS", "The title is empty.");
    const convo = (await ctx.db.unsafe.get("Conversation", args.conversationId)) as { userId: string } | null;
    if (!convo || convo.userId !== ctx.auth.userId) throw ctx.error("NOT_FOUND", "Conversation not found.");
    // Conversation updates are denied to clients by policy; ownership is
    // checked above.
    await ctx.db.unsafe.update("Conversation", args.conversationId, { title });
    return { ok: true, title };
  },
});
