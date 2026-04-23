import { mutation, v } from "@pylonsync/functions";

// Default props per block type. Keeps the "quick add" UX one-click — the
// user doesn't have to fill in anything before they see the block on the
// canvas.
const DEFAULTS: Record<string, Record<string, unknown>> = {
  // --- primitives ---
  heading: { level: 2, text: "New heading", align: "left" },
  text: { text: "Write something here…", align: "left" },
  button: { text: "Click me", href: "#", variant: "primary", align: "left" },
  image: {
    src: "https://images.unsplash.com/photo-1517842645767-c639042777db?w=900",
    alt: "",
    fit: "cover",
  },
  container: { layout: "stack", gap: 12, padding: 24, bg: "#ffffff" },
  divider: { margin: 16 },

  // --- sections: composed designs, each a single rich block ---
  "hero-centered": {
    eyebrow: "New — just launched",
    title: "The fastest way to ship your ideas.",
    subtitle: "A block-based editor with responsive breakpoints, reusable components, and realtime collaboration. Built for teams that move.",
    primaryCta: { text: "Get started", href: "#" },
    secondaryCta: { text: "See the demo", href: "#" },
  },
  "hero-split": {
    eyebrow: "For growing teams",
    title: "Design, publish, iterate.",
    subtitle: "Drop blocks onto a canvas, collaborate with cursors, publish with a click. No framework lock-in.",
    primaryCta: { text: "Start free", href: "#" },
    image: "https://images.unsplash.com/photo-1498050108023-c5249f4df085?w=1200&h=900&fit=crop",
  },
  "feature-grid": {
    eyebrow: "Built for teams",
    title: "Everything you need in one canvas.",
    // Feature icons are Lucide PascalCase names — the Icon component
    // translates them across libraries when Site.iconLibrary is set.
    items: [
      { icon: "Zap", title: "Realtime sync", text: "Every edit lands on every teammate's screen in milliseconds." },
      { icon: "Smartphone", title: "Responsive by default", text: "Desktop, tablet, phone — preview all three at once with Compare mode." },
      { icon: "Palette", title: "Design tokens", text: "Colors and type live as tokens. Change one, watch the whole site update." },
    ],
  },
  stats: {
    items: [
      { value: "2.3B", label: "Requests served" },
      { value: "99.99%", label: "Uptime last year" },
      { value: "< 40ms", label: "p95 latency" },
      { value: "200+", label: "Countries covered" },
    ],
  },
  "logo-cloud": {
    title: "Trusted by teams at",
    logos: ["Acme", "Globex", "Initech", "Stark", "Wayne", "Umbra"],
  },
  testimonial: {
    quote: "We shipped three landing pages in the time it used to take to design one. The compare mode alone saved our mobile launch.",
    author: "Jordan Ellis",
    role: "Head of Product, Riverline",
    avatar: "https://images.unsplash.com/photo-1494790108377-be9c29b29330?w=160&h=160&fit=crop&crop=faces",
  },
  pricing: {
    eyebrow: "Pricing",
    title: "Simple, scales with you.",
    tiers: [
      { name: "Starter", price: "$0", period: "forever", features: ["1 site", "5 pages", "Community support"], cta: "Start free", highlight: false },
      { name: "Pro", price: "$19", period: "per month", features: ["Unlimited sites", "Team collaboration", "Custom domains", "Priority support"], cta: "Start trial", highlight: true },
      { name: "Team", price: "$49", period: "per month", features: ["Everything in Pro", "SSO + audit log", "Role-based access", "Dedicated CSM"], cta: "Contact sales", highlight: false },
    ],
  },
  "cta-banner": {
    title: "Ready to ship something good?",
    subtitle: "Start free — no credit card required.",
    primaryCta: { text: "Create your first site", href: "#" },
  },
  faq: {
    title: "Frequently asked questions",
    items: [
      { q: "Can I use my own domain?", a: "Yes. Custom domains are included on Pro and Team plans. Map any domain through your DNS and Stage handles certificates automatically." },
      { q: "How does realtime collaboration work?", a: "Every mutation streams through a persistent change log. Teammates see edits land within 100ms — no setup, no merge conflicts." },
      { q: "Can I export the site?", a: "Absolutely. Every site can be exported as static HTML + assets, or connected to a CI pipeline that builds on push." },
      { q: "Is there an AI assistant?", a: "Planned. The DESIGN.md format is already aligned with coding-agent standards so AI tools can respect your design system out of the box." },
    ],
  },
  footer: {
    tagline: "Build. Ship. Iterate.",
    columns: [
      { title: "Product", links: [{ text: "Features", href: "#" }, { text: "Pricing", href: "#" }, { text: "Changelog", href: "#" }] },
      { title: "Company", links: [{ text: "About", href: "#" }, { text: "Careers", href: "#" }, { text: "Press", href: "#" }] },
      { title: "Resources", links: [{ text: "Docs", href: "#" }, { text: "Blog", href: "#" }, { text: "Community", href: "#" }] },
    ],
    copyright: "© 2026 Stage. All rights reserved.",
  },
};

export default mutation({
  args: {
    pageId: v.string(),
    type: v.string(),
    parentId: v.optional(v.string()),
    afterSort: v.optional(v.number()),
    propsJson: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org first");
    const page = await ctx.db.get("Page", args.pageId);
    if (!page || (page as { orgId?: string }).orgId !== ctx.auth.tenantId)
      throw ctx.error("NOT_FOUND", "page not found");

    if (!DEFAULTS[args.type]) throw ctx.error("INVALID_TYPE", `unknown block type: ${args.type}`);

    const propsJson = args.propsJson ?? JSON.stringify(DEFAULTS[args.type]);

    // Sort calculation — place new block right after `afterSort` when
    // provided (used for "insert between" UX), otherwise append at the
    // end of the page / parent. Using fractional sort keeps reorders
    // O(1) without rewriting every sibling — the inverse Linear/
    // Attio/etc pattern.
    const siblings = await ctx.db.query("Block", {
      pageId: args.pageId,
      parentId: args.parentId ?? null,
    });
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
      parentId: args.parentId ?? null,
      sort: newSort,
      type: args.type,
      propsJson,
      createdAt: new Date().toISOString(),
    });
    return { id };
  },
});
