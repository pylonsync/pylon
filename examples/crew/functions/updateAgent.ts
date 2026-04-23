import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    agentId: v.string(),
    name: v.optional(v.string()),
    role: v.optional(v.string()),
    systemPrompt: v.optional(v.string()),
    model: v.optional(v.string()),
    avatarEmoji: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");

    const agent = await ctx.db.get("Agent", args.agentId);
    if (!agent) throw ctx.error("NOT_FOUND", "agent not found");
    if ((agent as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("FORBIDDEN", "not in this org");

    const patch: Record<string, unknown> = {};
    if (args.name !== undefined) patch.name = args.name.trim();
    if (args.role !== undefined) patch.role = args.role.trim();
    if (args.systemPrompt !== undefined) patch.systemPrompt = args.systemPrompt;
    if (args.model !== undefined) patch.model = args.model;
    if (args.avatarEmoji !== undefined) patch.avatarEmoji = args.avatarEmoji;

    await ctx.db.update("Agent", args.agentId, patch);
    return { updated: true };
  },
});
