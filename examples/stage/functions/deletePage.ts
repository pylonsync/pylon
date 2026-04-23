import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { pageId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const page = await ctx.db.get("Page", args.pageId);
    if (!page || (page as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "page not found");

    // Refuse to leave a site with zero pages — the editor needs at least
    // one surface. Let the caller create a replacement first.
    const siblings = await ctx.db.query("Page", {
      siteId: (page as { siteId: string }).siteId,
    });
    if (siblings.length <= 1) throw ctx.error("LAST_PAGE", "a site must have at least one page");

    // Cascade: remove this page's blocks too.
    const blocks = await ctx.db.query("Block", { pageId: args.pageId });
    for (const b of blocks as { id: string }[]) {
      await ctx.db.delete("Block", b.id);
    }
    await ctx.db.delete("Page", args.pageId);
    return { deleted: true };
  },
});
