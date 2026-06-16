import { mutation } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";
import { slugify } from "../lib/agency";

// seedProjects — create the portfolio from config on first visit (idempotent).
// The public pages call it on mount; once any Project row exists it's a no-op,
// so it's safe to call on every load. A lock keeps two concurrent first-visits
// from double-seeding.
//
// Public so an anonymous first visitor seeds it: Projects are public marketing
// content (no PII), and this only ever writes the config's case studies.
export default mutation<Record<string, never>, { seeded: number }>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("agency_seed_projects");
    const existing = await ctx.db.unsafe.list("Project");
    if (existing.length > 0) return { seeded: 0 };

    const now = new Date().toISOString();
    let order = 0;
    for (const p of siteConfig.work.items) {
      await ctx.db.unsafe.insert("Project", {
        title: p.title,
        slug: p.slug || slugify(p.title),
        client: p.client,
        summary: p.summary,
        year: p.year ?? null,
        tags: p.tags.join(", "),
        selected: p.selected ?? false,
        published: true,
        order: order++,
        challenge: p.challenge ?? null,
        approach: p.approach ?? null,
        outcome: p.outcome ?? null,
        liveUrl: p.liveUrl ?? null,
        createdAt: now,
      });
    }
    return { seeded: order };
  },
});
