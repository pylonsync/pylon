import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    name: v.string(),
    role: v.string(),
    systemPrompt: v.string(),
    model: v.string(),
    avatarEmoji: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name required");

    const id = await ctx.db.insert("Agent", {
      orgId: ctx.auth.tenantId,
      name,
      role: args.role.trim(),
      systemPrompt: args.systemPrompt,
      model: args.model,
      avatarEmoji: args.avatarEmoji || "\u{1F916}",
      tools: null,
      createdBy: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
