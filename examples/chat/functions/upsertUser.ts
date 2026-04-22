import { mutation, v } from "@statecraft/functions";

/**
 * Idempotent user creation for the demo sign-in flow. Real apps route
 * users through magic-code or OAuth; this function exists so the sample
 * UI can "sign in" without configuring an email provider.
 *
 * NOT SAFE FOR PRODUCTION — anyone who knows an email can claim that
 * user. Lock this behind admin auth or delete it before shipping.
 */

const PALETTE = [
  "#ef4444", "#f97316", "#eab308", "#22c55e",
  "#06b6d4", "#6366f1", "#a855f7", "#ec4899",
];

export default mutation({
  args: {
    email: v.string(),
    displayName: v.string(),
  },
  async handler(ctx, args) {
    const email = args.email.trim().toLowerCase();
    if (!email.includes("@")) throw ctx.error("INVALID_EMAIL", "email must contain @");

    const existing = await ctx.db.lookup("User", "email", email);
    if (existing) {
      return existing;
    }

    // Deterministic avatar color so the same user always renders consistently
    // across reloads. Real apps would store a real upload.
    const hash = email
      .split("")
      .reduce((acc, ch) => (acc * 31 + ch.charCodeAt(0)) | 0, 0);
    const avatarColor = PALETTE[Math.abs(hash) % PALETTE.length];

    const id = await ctx.db.insert("User", {
      email,
      displayName: args.displayName.trim() || email.split("@")[0],
      avatarColor,
      createdAt: new Date().toISOString(),
    });
    return await ctx.db.get("User", id);
  },
});
