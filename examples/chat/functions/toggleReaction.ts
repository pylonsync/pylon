import { mutation, v } from "@pylonsync/functions";

/**
 * Toggle an emoji reaction on a message. If the user has already reacted
 * with this emoji, remove it; otherwise add it. One atomic write so a
 * rapid double-tap can't end up with two copies of the same reaction.
 *
 * Relies on the `(messageId, userId, emoji)` unique index to make
 * duplicates impossible at the DB level even if the check races.
 */
export default mutation({
  args: {
    messageId: v.id("Message"),
    emoji: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to react");

    // Normalize emoji key so ":thumbsup:" and "👍" don't collide. Accept both
    // shortcodes (matching Slack's API) and literal emoji. Clamp to 32 chars
    // to keep the index small.
    const emoji = args.emoji.slice(0, 32);
    if (emoji.length === 0) throw ctx.error("EMPTY_EMOJI", "emoji required");

    const message = await ctx.db.get("Message", args.messageId);
    if (!message) throw ctx.error("MESSAGE_NOT_FOUND", "message does not exist");

    const existing = await ctx.db.query("Reaction", {
      messageId: args.messageId,
      userId: ctx.auth.userId,
      emoji,
    });

    if (existing.length > 0) {
      await ctx.db.delete("Reaction", existing[0].id);
      return { action: "removed" as const, reactionId: existing[0].id };
    }

    const reactionId = await ctx.db.insert("Reaction", {
      messageId: args.messageId,
      userId: ctx.auth.userId,
      emoji,
      createdAt: new Date().toISOString(),
    });
    return { action: "added" as const, reactionId };
  },
});
