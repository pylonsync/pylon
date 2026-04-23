import { mutation, v } from "@pylonsync/functions";

// Save the user-edited design-token JSON on a Site. Clients pass the
// whole token object — simpler than field-by-field updates and there's
// rarely more than a few dozen keys. The renderer on both the canvas
// and the public preview reads tokensJson to drive CSS variables.
export default mutation({
  args: {
    siteId: v.string(),
    tokensJson: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const site = await ctx.db.get("Site", args.siteId);
    if (!site || (site as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "site not found");
    try { JSON.parse(args.tokensJson); } catch { throw ctx.error("INVALID_JSON", "tokensJson must be valid JSON"); }
    await ctx.db.update("Site", args.siteId, { tokensJson: args.tokensJson });
    return { ok: true };
  },
});
