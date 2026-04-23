import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    blockId: v.string(),
    propsJson: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const block = await ctx.db.get("Block", args.blockId);
    if (!block || (block as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "block not found");
    // Parse to validate the payload is real JSON before writing it.
    try { JSON.parse(args.propsJson); }
    catch { throw ctx.error("INVALID_PROPS", "propsJson must be valid JSON"); }
    await ctx.db.update("Block", args.blockId, { propsJson: args.propsJson });
    return { ok: true };
  },
});
