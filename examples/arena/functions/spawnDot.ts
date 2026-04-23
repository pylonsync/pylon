import { mutation, v } from "@pylonsync/functions";

const PALETTE = [
  "#f5b946", "#7ab7ff", "#5ee6a6", "#c89dff",
  "#ff6b9d", "#ffd166", "#80e0d8", "#b48cff",
];

/**
 * Create a dot for the caller. Idempotent — if a dot already exists
 * for this userId we return its id without creating a duplicate.
 * Bots are separate rows with isBot = true.
 */
export default mutation({
  args: {
    userId: v.string(),
    label: v.optional(v.string()),
    isBot: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const existing = await ctx.db.query("Dot", { userId: args.userId });
    if (existing.length > 0) return { id: existing[0].id as string };

    const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
    const id = await ctx.db.insert("Dot", {
      userId: args.userId,
      x: Math.random(),
      y: Math.random(),
      tx: Math.random(),
      ty: Math.random(),
      color,
      label: args.label ?? null,
      speed: 0.12,
      isBot: args.isBot ?? false,
      lastSeenAt: new Date().toISOString(),
    });
    return { id };
  },
});
