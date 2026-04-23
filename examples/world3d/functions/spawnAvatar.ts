import { mutation, v } from "@pylonsync/functions";

const PALETTE = [
  "#8b5cf6", "#f5b946", "#7ab7ff", "#5ee6a6",
  "#ff6b9d", "#ffd166", "#80e0d8", "#c89dff",
];

const NAMES = [
  "nova", "onyx", "echo", "lyra", "atlas", "rhea",
  "orion", "vega", "juno", "mira", "zed", "kai",
];

function randomName() {
  return `${NAMES[Math.floor(Math.random() * NAMES.length)]}_${Math.floor(Math.random() * 900 + 100)}`;
}

export default mutation({
  args: {
    userId: v.string(),
    name: v.optional(v.string()),
    isBot: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const existing = await ctx.db.query("Avatar", { userId: args.userId });
    if (existing.length > 0) return { id: existing[0].id as string };

    const color = PALETTE[Math.floor(Math.random() * PALETTE.length)];
    const name = args.name ?? randomName();
    const id = await ctx.db.insert("Avatar", {
      userId: args.userId,
      name,
      color,
      // Spread avatars around the center so they don't stack.
      x: (Math.random() - 0.5) * 12,
      y: 0,
      z: (Math.random() - 0.5) * 12,
      heading: Math.random() * Math.PI * 2,
      emote: null,
      isBot: args.isBot ?? false,
      lastSeenAt: new Date().toISOString(),
    });
    return { id };
  },
});
