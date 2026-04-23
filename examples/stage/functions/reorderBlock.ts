import { mutation, v } from "@pylonsync/functions";

// Drop a block into an explicit slot. Callers compute the target sort
// (usually the midpoint between two neighbors — client does this for
// drag-to-reorder) and pass it here. Keeps the server dumb about layout
// semantics; one fractional-sort field is enough for any position.
export default mutation({
  args: {
    blockId: v.string(),
    newSort: v.number(),
    newParentId: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const block = await ctx.db.get("Block", args.blockId);
    if (!block || (block as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "block not found");

    const patch: Record<string, unknown> = { sort: args.newSort };
    if (args.newParentId !== undefined) {
      patch.parentId = args.newParentId || null;
    }
    await ctx.db.update("Block", args.blockId, patch);
    return { ok: true };
  },
});
