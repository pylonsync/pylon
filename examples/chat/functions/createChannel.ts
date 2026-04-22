import { mutation, v } from "@statecraft/functions";

/**
 * Create a channel in the caller's workspace. The creator auto-joins as
 * admin so they can manage it, and for private channels they're the only
 * initial member.
 */
export default mutation({
  args: {
    name: v.string(),
    topic: v.optional(v.string()),
    isPrivate: v.boolean(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in to create a channel");

    // Channel names follow Slack conventions: lowercase, alphanumeric + dashes,
    // 1–80 chars, no spaces. A loose check here keeps `#foo bar` from becoming
    // a channel — the UI would choke on it and DMs would be ambiguous.
    const name = args.name.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{0,79}$/.test(name)) {
      throw ctx.error(
        "INVALID_NAME",
        "channel name must be lowercase alphanumeric + dashes, max 80 chars",
      );
    }

    const now = new Date().toISOString();
    const channelId = await ctx.db.insert("Channel", {
      name,
      topic: args.topic ?? "",
      isPrivate: args.isPrivate,
      createdBy: ctx.auth.userId,
      createdAt: now,
    });

    // Auto-join creator as admin. The tenant_scope plugin stamps tenantId
    // for us so we don't pass it explicitly.
    await ctx.db.insert("Membership", {
      channelId,
      userId: ctx.auth.userId,
      role: "admin",
      joinedAt: now,
    });

    return { channelId };
  },
});
