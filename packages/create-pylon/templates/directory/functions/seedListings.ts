import { mutation } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";

// seedListings — load the starter directory from config on first visit
// (idempotent). The browse island calls this on mount; once any Listing exists
// it's a no-op, so it's safe to call on every load. A lock keeps two concurrent
// first-visits from double-seeding.
//
// Public so an anonymous first visitor seeds the directory — it only writes the
// config's PII-free starter entries, never reads or returns anything sensitive.
export default mutation<Record<string, never>, { seeded: boolean; count: number }>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("directory_seed_listings");
    const existing = await ctx.db.unsafe.list("Listing");
    if (existing.length > 0) return { seeded: false, count: existing.length };

    for (const l of siteConfig.seedListings) {
      await ctx.db.unsafe.insert("Listing", {
        name: l.name,
        tagline: l.tagline,
        url: l.url,
        category: l.category,
        tags: l.tags ?? null,
        description: l.description ?? null,
        votes: l.votes ?? 0,
        featured: l.featured ?? false,
        createdAt: new Date().toISOString(),
      });
    }
    return { seeded: true, count: siteConfig.seedListings.length };
  },
});
