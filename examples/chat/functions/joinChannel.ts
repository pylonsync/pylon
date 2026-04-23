import { mutation, v } from "@pylonsync/functions";

/**
 * Join a channel as a member. Idempotent — calling twice is fine.
 *
 * Public channels: anyone in the workspace can join.
 * Private channels: only an existing admin can add a member (via
 * `args.userId`); self-join is rejected.
 */
export default mutation({
  args: {
    channelId: v.id("Channel"),
    // When inviting someone else to a private channel, pass their user id.
    // Omit to self-join a public channel.
    userId: v.optional(v.id("User")),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to join channels");

    const channel = await ctx.db.get("Channel", args.channelId);
    if (!channel) throw ctx.error("CHANNEL_NOT_FOUND", "channel does not exist");

    const targetUserId = args.userId ?? ctx.auth.userId;
    const isSelf = targetUserId === ctx.auth.userId;

    if (channel.isPrivate) {
      if (isSelf) {
        throw ctx.error(
          "PRIVATE_CHANNEL",
          "private channels require an existing member to invite you",
        );
      }
      // Caller must be an admin of this channel.
      const callerMembership = await ctx.db.query("Membership", {
        channelId: args.channelId,
        userId: ctx.auth.userId,
      });
      if (callerMembership.length === 0 || callerMembership[0].role !== "admin") {
        throw ctx.error("FORBIDDEN", "only channel admins can invite to private channels");
      }
    }

    // Idempotency: if already a member, return the existing membership id
    // rather than erroring. Double-clicks + retries don't break the UI.
    const existing = await ctx.db.query("Membership", {
      channelId: args.channelId,
      userId: targetUserId,
    });
    if (existing.length > 0) {
      return { membershipId: existing[0].id, joined: false };
    }

    const membershipId = await ctx.db.insert("Membership", {
      channelId: args.channelId,
      userId: targetUserId,
      role: "member",
      joinedAt: new Date().toISOString(),
    });

    return { membershipId, joined: true };
  },
});
