import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { email: v.string(), displayName: v.string() },
  async handler(ctx, args) {
    const email = args.email.trim().toLowerCase();
    if (!email) throw ctx.error("INVALID_EMAIL", "email required");
    const name = args.displayName.trim();
    if (!name) throw ctx.error("INVALID_NAME", "display name required");

    const existing = await ctx.db.query("User", { email });
    if (existing.length > 0) return existing[0];

    const palette = [
      "#8b5cf6", "#6366f1", "#3b82f6", "#06b6d4", "#10b981",
      "#84cc16", "#eab308", "#f97316", "#ef4444", "#ec4899",
    ];
    let hash = 0;
    for (let i = 0; i < email.length; i++) hash = (hash * 31 + email.charCodeAt(i)) | 0;
    const avatarColor = palette[Math.abs(hash) % palette.length];
    const id = await ctx.db.insert("User", {
      email, displayName: name, avatarColor,
      createdAt: new Date().toISOString(),
    });
    return await ctx.db.get("User", id);
  },
});
