import { mutation, v } from "@pylonsync/functions";

// Toggling `publishedAt` gates whether the public /p/:slug preview renders.
// Everything else (pages, blocks) is always "current" — there's no draft
// vs. published fork in v1. Publish is a single-bit decision: is this
// site visible to the world yet?
export default mutation({
  args: { siteId: v.string(), publish: v.boolean() },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const site = await ctx.db.get("Site", args.siteId);
    if (!site || (site as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "site not found");
    await ctx.db.update("Site", args.siteId, {
      publishedAt: args.publish ? new Date().toISOString() : null,
    });
    return { ok: true };
  },
});
