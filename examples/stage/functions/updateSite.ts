import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    siteId: v.string(),
    name: v.optional(v.string()),
    accentColor: v.optional(v.string()),
    typeface: v.optional(v.string()),
    faviconEmoji: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const site = await ctx.db.get("Site", args.siteId);
    if (!site || (site as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "site not found");

    const patch: Record<string, unknown> = {};
    if (args.name !== undefined) patch.name = args.name.trim();
    if (args.accentColor !== undefined) patch.accentColor = args.accentColor;
    if (args.typeface !== undefined) patch.typeface = args.typeface;
    if (args.faviconEmoji !== undefined) patch.faviconEmoji = args.faviconEmoji;
    await ctx.db.update("Site", args.siteId, patch);
    return { ok: true };
  },
});
