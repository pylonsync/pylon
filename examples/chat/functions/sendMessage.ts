import { mutation, v } from "@pylonsync/functions";

/**
 * Send a message to a channel.
 *
 * Transactional: membership check + message insert happen in one write tx,
 * so a user that loses membership between the check and the insert can't
 * sneak a message in. Rolls back on any error.
 *
 * All clients subscribed to the channel get the message via the sync engine
 * — no explicit broadcast needed; the change log fan-out handles it.
 */
export default mutation({
  args: {
    channelId: v.id("Channel"),
    body: v.string(),
    parentMessageId: v.optional(v.id("Message")),
    // Threaded through by `useMutation({ optimistic })` on the client.
    // When present, the runtime uses it as the row id so the canonical
    // insert matches the optimistic ghost the client painted — the WS
    // broadcast lands as a field-level merge instead of replacing the
    // ghost row, and there's no flash. Optional + ignored if absent.
    _optimisticId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to send messages");

    const trimmed = args.body.trim();
    if (trimmed.length === 0) throw ctx.error("EMPTY_MESSAGE", "message body cannot be empty");
    if (trimmed.length > 4000) throw ctx.error("MESSAGE_TOO_LONG", "message exceeds 4000 characters");

    const channel = await ctx.db.get("Channel", args.channelId);
    if (!channel) throw ctx.error("CHANNEL_NOT_FOUND", "channel does not exist");

    // Private channels require explicit membership. Public channels allow
    // any workspace member — we rely on the tenant_scope plugin to bound
    // access to the workspace, then only check membership for private.
    if (channel.isPrivate) {
      const memberships = await ctx.db.query("Membership", {
        channelId: args.channelId,
        userId: ctx.auth.userId,
      });
      if (memberships.length === 0) {
        throw ctx.error("FORBIDDEN", "not a member of this private channel");
      }
    }

    // Thread parents must exist in the same channel — reject cross-channel
    // thread hijacking.
    if (args.parentMessageId) {
      const parent = await ctx.db.get("Message", args.parentMessageId);
      if (!parent) throw ctx.error("PARENT_NOT_FOUND", "parent message not found");
      if (parent.channelId !== args.channelId) {
        throw ctx.error("PARENT_WRONG_CHANNEL", "parent message is in a different channel");
      }
    }

    const messageId = await ctx.db.insert("Message", {
      // Honor the client's optimistic id when present; the runtime
      // validates it's a 40-char hex (the shape generateId() produces)
      // and falls back to its own generator if absent or malformed.
      ...(args._optimisticId ? { id: args._optimisticId } : {}),
      // tenantId is auto-stamped by the tenant_scope plugin from auth.tenantId
      channelId: args.channelId,
      authorId: ctx.auth.userId,
      parentMessageId: args.parentMessageId ?? null,
      body: trimmed,
      createdAt: new Date().toISOString(),
    });

    return { messageId };
  },
});
