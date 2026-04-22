import { mutation, v } from "@statecraft/functions";

/**
 * Open (or reuse) a direct-message channel between the caller and one other
 * user. DMs are modeled as private two-member Channels with a deterministic
 * name — sort both user ids and join with `dm:` so the pair maps to a
 * single channel regardless of who initiates. That makes "did a DM exist
 * already?" a single lookup instead of an n-way scan of memberships.
 */
export default mutation({
  args: {
    otherUserId: v.id("User"),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (args.otherUserId === ctx.auth.userId) {
      throw ctx.error("CANT_DM_SELF", "cannot DM yourself");
    }

    // Deterministic channel name. Sorting avoids creating two DM channels
    // for the same pair (A→B vs B→A).
    const [a, b] = [ctx.auth.userId, args.otherUserId].sort();
    const name = `dm:${a}:${b}`;

    const existing = await ctx.db.query("Channel", { name });
    if (existing.length > 0) {
      return { channelId: existing[0].id };
    }

    // Verify the other user exists — avoid creating DM channels to ghosts.
    const other = await ctx.db.get("User", args.otherUserId);
    if (!other) throw ctx.error("USER_NOT_FOUND", "user does not exist");

    const now = new Date().toISOString();
    const channelId = await ctx.db.insert("Channel", {
      name,
      topic: "",
      isPrivate: true,
      createdBy: ctx.auth.userId,
      createdAt: now,
    });

    // Both participants get Membership rows so the private-channel
    // membership check in sendMessage passes for both.
    await ctx.db.insert("Membership", {
      channelId,
      userId: ctx.auth.userId,
      role: "member",
      joinedAt: now,
    });
    await ctx.db.insert("Membership", {
      channelId,
      userId: args.otherUserId,
      role: "member",
      joinedAt: now,
    });

    return { channelId };
  },
});
