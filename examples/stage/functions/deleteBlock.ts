import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { blockId: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const block = await ctx.db.get("Block", args.blockId);
    if (!block || (block as { orgId?: string }).orgId !== ctx.auth.tenantId)
      return { deleted: false };
    // Cascade: if this is a container, drop its children too.
    const children = await ctx.db.query("Block", { parentId: args.blockId });
    for (const c of children as { id: string }[]) {
      await ctx.db.delete("Block", c.id);
    }
    await ctx.db.delete("Block", args.blockId);
    return { deleted: true };
  },
});
