import { mutation, v } from "@pylonsync/functions";
import {
  BIO_MAX,
  DISPLAY_NAME_MAX,
  isPostImageUrl,
  normalizeUsername,
  usernameProblem,
} from "../lib/social";

/**
 * Change the caller's username, name, bio, or avatar. A function rather
 * than a client update so the username rules and uniqueness are checked on
 * the server; the Profile policy refuses direct writes.
 */
export default mutation<
  { username: string; displayName: string; bio?: string; avatarUrl?: string },
  { username: string }
>({
  // Guest sessions count: a visitor can post without signing up.
  auth: "guest",
  args: {
    username: v.string(),
    displayName: v.string(),
    bio: v.optional(v.string()),
    // An empty string removes the photo.
    avatarUrl: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("AUTH_REQUIRED", "Open the app to start a session first.");
    const [profile] = await ctx.db.query("Profile", { userId, $limit: 1 });
    if (!profile) throw ctx.error("NOT_FOUND", "Open the app once to create your profile first.");

    const username = normalizeUsername(args.username);
    const problem = usernameProblem(username);
    if (problem) throw ctx.error("INVALID_USERNAME", problem);
    if (username !== profile.username) {
      await ctx.db.advisoryLock(`username:${username}`);
      const [taken] = await ctx.db.query("Profile", { username, $limit: 1 });
      if (taken) throw ctx.error("USERNAME_TAKEN", `@${username} is taken.`);
    }

    const displayName = args.displayName.trim();
    if (!displayName) throw ctx.error("INVALID_NAME", "Add a name.");
    if (displayName.length > DISPLAY_NAME_MAX) throw ctx.error("INVALID_NAME", `Use at most ${DISPLAY_NAME_MAX} characters for the name.`);
    const bio = (args.bio ?? "").trim();
    if (bio.length > BIO_MAX) throw ctx.error("INVALID_BIO", `Use at most ${BIO_MAX} characters for the bio.`);
    const avatarUrl = args.avatarUrl ? args.avatarUrl.trim() : null;
    if (avatarUrl && !isPostImageUrl(avatarUrl)) throw ctx.error("INVALID_IMAGE", "Upload the photo again.");

    // The Profile policy refuses client writes; this function is the
    // checked path, so it writes past the policy on purpose.
    await ctx.db.unsafe.update("Profile", String(profile.id), {
      username,
      displayName,
      bio: bio || null,
      avatarUrl,
    });
    return { username };
  },
});
