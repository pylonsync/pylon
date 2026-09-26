import { mutation, v } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";
import { titleFromMessage } from "../lib/chat";

// Start a conversation titled from its first message. Guests may call it, so a
// visitor can chat before signing in. `respond` then adds the messages.
export default mutation({
  auth: "guest",
  args: { firstMessage: v.string() },
  async handler(ctx, args) {
    const text = args.firstMessage.trim();
    if (text.length === 0) throw ctx.error("INVALID_ARGS", "The message is empty.");
    if (text.length > siteConfig.chat.maxInputChars) {
      throw ctx.error("INVALID_ARGS", "The message is too long.");
    }
    const now = new Date().toISOString();
    // Conversation inserts are denied to clients by policy; this function is
    // the one place that creates them, stamped with the caller's id.
    const id = await ctx.db.unsafe.insert("Conversation", {
      userId: ctx.auth.userId,
      title: titleFromMessage(text),
      example: false,
      createdAt: now,
      updatedAt: now,
    });
    return { id };
  },
});
