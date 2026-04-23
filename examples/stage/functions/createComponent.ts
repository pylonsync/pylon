import { mutation, v } from "@pylonsync/functions";

// Promote a block subtree into a reusable Component. The source block
// (and its descendants) get cloned into a "master" tree under the new
// Component — the originals are then replaced with a single
// `component` instance block that points at the Component.
//
// v1 only handles promoting a top-level block (no nested children);
// extending to full subtree clone is additive and doesn't change the
// entity shape.
export default mutation({
  args: {
    siteId: v.string(),
    name: v.string(),
    fromBlockId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const site = await ctx.db.get("Site", args.siteId);
    if (!site || (site as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "site not found");
    const name = args.name.trim();
    if (!name) throw ctx.error("INVALID_NAME", "name required");

    const now = new Date().toISOString();
    const componentId = await ctx.db.insert("Component", {
      orgId: ctx.auth.tenantId,
      siteId: args.siteId,
      name,
      createdBy: ctx.auth.userId,
      createdAt: now,
    });

    if (args.fromBlockId) {
      const source = await ctx.db.get("Block", args.fromBlockId);
      if (source && (source as { orgId?: string }).orgId === ctx.auth.tenantId) {
        const src = source as {
          type: string; propsJson: string;
          pageId?: string; parentId?: string; sort: number; siteId: string;
        };
        // Clone the source into the component's master tree (no pageId).
        await ctx.db.insert("Block", {
          orgId: ctx.auth.tenantId,
          siteId: args.siteId,
          pageId: null,
          parentId: null,
          componentId,
          sort: 0,
          type: src.type,
          propsJson: src.propsJson,
          createdAt: now,
        });
        // Swap the original block for an instance of the component.
        await ctx.db.update("Block", args.fromBlockId, {
          type: "component",
          componentId,
          propsJson: "{}",
        });
      }
    } else {
      // Seed with an empty heading so the component has something to edit.
      await ctx.db.insert("Block", {
        orgId: ctx.auth.tenantId,
        siteId: args.siteId,
        pageId: null,
        parentId: null,
        componentId,
        sort: 0,
        type: "heading",
        propsJson: JSON.stringify({ level: 2, text: name, align: "left" }),
        createdAt: now,
      });
    }

    return { componentId };
  },
});
