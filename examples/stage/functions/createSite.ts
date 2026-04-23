import { mutation, v } from "@pylonsync/functions";

// Seed blocks so a fresh site looks like a real landing page instead of a
// blank canvas. Same pattern as the other examples' starter data.
const STARTER_BLOCKS: Array<{ type: string; props: Record<string, unknown> }> = [
  {
    type: "heading",
    props: { level: 1, text: "Launch something good this week.", align: "center" },
  },
  {
    type: "text",
    props: {
      text: "Stage turns ideas into shippable pages in minutes. Drop in headings, text, buttons, and images — publish when it's ready.",
      align: "center",
    },
  },
  {
    type: "button",
    props: { text: "Get started →", href: "#", variant: "primary", align: "center" },
  },
];

export default mutation({
  args: {
    name: v.string(),
    slug: v.string(),
    accentColor: v.optional(v.string()),
    faviconEmoji: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name required");
    const slug = args.slug.trim().toLowerCase();
    if (!/^[a-z0-9][a-z0-9-]{1,60}$/.test(slug))
      throw ctx.error("INVALID_SLUG", "lowercase letters/numbers/dashes, 2–60");

    const now = new Date().toISOString();
    let siteId: string;
    try {
      siteId = await ctx.db.insert("Site", {
        orgId: ctx.auth.tenantId,
        name,
        slug,
        faviconEmoji: args.faviconEmoji || "\u{2728}",
        accentColor: args.accentColor || "#ec4899",
        typeface: "sans",
        createdBy: ctx.auth.userId,
        createdAt: now,
        publishedAt: null,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (/UNIQUE constraint/i.test(msg))
        throw ctx.error("SLUG_TAKEN", `slug "${slug}" is taken`);
      throw e;
    }

    // Home page seeded so the editor has a starting surface. Additional
    // pages are opt-in.
    const pageId = await ctx.db.insert("Page", {
      orgId: ctx.auth.tenantId,
      siteId,
      slug: "/",
      title: "Home",
      sort: 0,
      metaTitle: null,
      metaDescription: null,
      createdAt: now,
    });

    for (let i = 0; i < STARTER_BLOCKS.length; i++) {
      const b = STARTER_BLOCKS[i];
      await ctx.db.insert("Block", {
        orgId: ctx.auth.tenantId,
        siteId,
        pageId,
        parentId: null,
        sort: i,
        type: b.type,
        propsJson: JSON.stringify(b.props),
        createdAt: now,
      });
    }

    return { siteId, pageId };
  },
});
