import { mutation, v } from "@pylonsync/functions";

// Drop a Component instance onto a page. Renders the component's
// master tree — edits to the master flow live to every instance.
export default mutation({
  args: {
    pageId: v.string(),
    componentId: v.string(),
    afterSort: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const page = await ctx.db.get("Page", args.pageId);
    if (!page || (page as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "page not found");
    const comp = await ctx.db.get("Component", args.componentId);
    if (!comp || (comp as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "component not found");

    const siblings = await ctx.db.query("Block", { pageId: args.pageId, parentId: null });
    const sorts = siblings.map((b) => b.sort as number).sort((a, b) => a - b);
    let newSort: number;
    if (args.afterSort !== undefined) {
      const nextHigher = sorts.find((s) => s > (args.afterSort as number));
      newSort = nextHigher !== undefined
        ? ((args.afterSort as number) + nextHigher) / 2
        : (args.afterSort as number) + 1;
    } else {
      newSort = sorts.length ? sorts[sorts.length - 1] + 1 : 0;
    }

    const id = await ctx.db.insert("Block", {
      orgId: ctx.auth.tenantId,
      siteId: (page as { siteId: string }).siteId,
      pageId: args.pageId,
      parentId: null,
      componentId: args.componentId,
      sort: newSort,
      type: "component",
      propsJson: "{}",
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
