import { mutation, v } from "@statecraft/functions";

/**
 * Update the caller's read marker for a channel. Upserts — one marker per
 * (user, channel). The UI pings this when the user opens or scrolls to the
 * bottom of a channel.
 *
 * Badges + unread counts on the client compare a message's createdAt
 * against the marker's lastReadAt — anything after it is unread.
 */
export default mutation({
  args: {
    channelId: v.id("Channel"),
    // Omit to mark "read through now". Pass a timestamp to mark read up to
    // a specific message time (useful for desktop apps that defer batches).
    at: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to mark read");

    const lastReadAt = args.at ?? new Date().toISOString();

    const existing = await ctx.db.query("ReadMarker", {
      userId: ctx.auth.userId,
      channelId: args.channelId,
    });

    if (existing.length > 0) {
      // Only move the marker forward. A slow render that marks an older
      // timestamp shouldn't regress the pointer and bring back unread
      // badges the user already dismissed.
      const prev = existing[0];
      if (new Date(prev.lastReadAt).getTime() >= new Date(lastReadAt).getTime()) {
        return { markerId: prev.id, unchanged: true };
      }
      await ctx.db.update("ReadMarker", prev.id, { lastReadAt });
      return { markerId: prev.id, unchanged: false };
    }

    const markerId = await ctx.db.insert("ReadMarker", {
      channelId: args.channelId,
      userId: ctx.auth.userId,
      lastReadAt,
    });
    return { markerId, unchanged: false };
  },
});
