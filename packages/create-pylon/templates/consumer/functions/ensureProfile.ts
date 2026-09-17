import { mutation } from "@pylonsync/functions";
import { starterUsername } from "../lib/social";

/**
 * The caller's profile, created on first use. Every page calls this once a
 * session exists, so a visitor has a username before they post, like, or
 * comment. A starter username is `visitor_xxxxxx`; the profile page can
 * change it.
 */
export default mutation<Record<string, never>, { id: string; username: string; created: boolean }>({
  // Guest sessions count: a visitor can post without signing up.
  auth: "guest",
  args: {},
  async handler(ctx) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Open the app to start a session first.");
    // Two tabs opening at once must not create two profiles.
    await ctx.db.advisoryLock(`profile:${userId}`);
    const [existing] = await ctx.db.query("Profile", { userId, $limit: 1 });
    if (existing) return { id: String(existing.id), username: String(existing.username), created: false };

    const base = starterUsername(userId);
    for (let attempt = 0; attempt < 5; attempt++) {
      const username = attempt === 0 ? base : `${base}${attempt}`;
      const [taken] = await ctx.db.query("Profile", { username, $limit: 1 });
      if (taken) continue;
      // The Profile policy refuses client writes; this function is the
      // checked path, so it writes past the policy on purpose.
      const id = await ctx.db.unsafe.insert("Profile", {
        userId,
        username,
        displayName: "Visitor",
        createdAt: new Date().toISOString(),
      });
      return { id, username, created: true };
    }
    throw ctx.error("USERNAME_UNAVAILABLE", "Could not pick a free username. Try again.");
  },
});
