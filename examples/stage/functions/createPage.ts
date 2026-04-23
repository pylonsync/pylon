import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    siteId: v.string(),
    title: v.string(),
    slug: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");

    const site = await ctx.db.get("Site", args.siteId);
    if (!site || (site as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "site not found");

    const title = args.title.trim();
    if (title.length === 0) throw ctx.error("INVALID_TITLE", "title required");
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9\-/]{0,60}$|^\/$/.test(slug))
      throw ctx.error("INVALID_SLUG", "lowercase letters/numbers/dashes/slashes");

    // Sort after every existing page so new ones append to the tree.
    const existing = await ctx.db.query("Page", { siteId: args.siteId });
    const maxSort = existing.reduce(
      (m, p) => Math.max(m, (p.sort as number) ?? 0),
      -1,
    );

    const now = new Date().toISOString();
    try {
      const id = await ctx.db.insert("Page", {
        orgId: ctx.auth.tenantId,
        siteId: args.siteId,
        slug,
        title,
        sort: maxSort + 1,
        metaTitle: null,
        metaDescription: null,
        createdAt: now,
      });
      return { id };
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" already exists on this site`);
      throw e;
    }
  },
});
