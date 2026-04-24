import { mutation, v } from "@pylonsync/functions";

/**
 * Find-or-create a User row keyed by email. Avatar color is seeded
 * deterministically from the email hash so repeat logins get the same
 * color without a lookup.
 */
export default mutation({
  args: { email: v.string(), displayName: v.string() },
  async handler(ctx, args) {
    const email = args.email.trim().toLowerCase();
    if (!email) throw ctx.error("INVALID_EMAIL", "email required");
    const displayName = args.displayName.trim();
    if (!displayName) throw ctx.error("INVALID_NAME", "display name required");

    // Find-or-update by email. If the row exists but was seeded by
    // /api/auth/magic/verify with displayName=email (the default when
    // verify creates a User before we know the caller's real name),
    // overwrite the display name on this call.
    const existing = await ctx.db.query("User", { email });
    if (existing.length > 0) {
      const row = existing[0] as Record<string, unknown>;
      if (displayName && row.displayName !== displayName) {
        await ctx.db.update("User", row.id as string, { displayName });
        return { ...row, displayName };
      }
      return row;
    }

    const palette = [
      "#8b5cf6", "#6366f1", "#3b82f6", "#06b6d4", "#10b981",
      "#84cc16", "#eab308", "#f97316", "#ef4444", "#ec4899",
    ];
    let hash = 0;
    for (let i = 0; i < email.length; i++) hash = (hash * 31 + email.charCodeAt(i)) | 0;
    const avatarColor = palette[Math.abs(hash) % palette.length];

    const id = await ctx.db.insert("User", {
      email,
      displayName,
      avatarColor,
      createdAt: new Date().toISOString(),
    });
    return await ctx.db.get("User", id);
  },
});
