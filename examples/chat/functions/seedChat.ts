import { mutation } from "@pylonsync/functions";

/**
 * Populate an empty workspace with a few starter channels and some opening
 * messages in #general, so a freshly signed-in user lands in a structured,
 * lived-in workspace instead of a blank sidebar. Idempotent: returns early if
 * any channel already exists. Messages are authored by the caller (the
 * workspace's first member setting things up).
 */
export default mutation({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const userId = ctx.auth.userId;

    const existing = await ctx.db.query("Channel", {});
    if (existing.length > 0) return { seeded: false, reason: "already has channels" };

    const now = Date.now();
    const iso = (offsetMin: number) => new Date(now + offsetMin * 60_000).toISOString();

    const channels = [
      { name: "general", topic: "Company-wide announcements and chatter" },
      { name: "engineering", topic: "Ship logs, PRs, and incident threads" },
      { name: "random", topic: "Non-work banter — memes, lunch, weekend plans" },
    ];
    const ids: Record<string, string> = {};
    for (const c of channels) {
      const channelId = await ctx.db.insert("Channel", {
        name: c.name,
        topic: c.topic,
        isPrivate: false,
        createdBy: userId,
        createdAt: iso(-60 * 24),
      });
      await ctx.db.insert("Membership", {
        channelId,
        userId,
        role: "admin",
        joinedAt: iso(-60 * 24),
      });
      ids[c.name] = channelId;
    }

    const opening = [
      "Welcome to the team workspace 👋 Everything syncs live across tabs and devices.",
      "Use #engineering for ship logs and PRs, and #random for everything else.",
      "Try opening this page in a second tab — messages and reactions sync instantly.",
    ];
    let i = 0;
    for (const body of opening) {
      i += 1;
      await ctx.db.insert("Message", {
        channelId: ids["general"],
        authorId: userId,
        parentMessageId: null,
        body,
        createdAt: iso(-60 * 24 + i),
      });
    }

    return { seeded: true, channels: channels.length, messages: opening.length };
  },
});
