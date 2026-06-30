import { mutation, v } from "@pylonsync/functions";

/**
 * Bulk-seed the catalog with ~10k synthetic products. Idempotent —
 * skips if the store is already populated. Runs on first launch of
 * the example so visitors land on a full-looking store page.
 *
 * Performance note: this whole handler is one mutation, so all the
 * inserts share a single transaction (the handler IS the transaction)
 * and pay the fsync once. ~10k rows × ~3ms each = 30s on an SSD; fine
 * for a dev seed. For larger bulk imports outside a function call,
 * use the HTTP `/api/transact` endpoint with batched ops.
 */
const BRANDS = [
  "Summit", "Orbit", "Nimbus", "Harbor", "Forge",
  "Atlas", "Quill", "Relay", "Vector", "Motif",
];

const CATEGORIES = [
  "Shoes", "Shirts", "Pants", "Jackets", "Hats",
  "Bags", "Watches", "Electronics", "Home", "Kitchen",
];

const COLORS = [
  "black", "white", "red", "blue", "green",
  "yellow", "gray", "navy", "olive", "burgundy",
];

const ADJECTIVES = [
  "lightweight", "rugged", "minimalist", "vintage", "technical",
  "heritage", "premium", "breathable", "waterproof", "seamless",
];

const NOUNS = [
  "cruiser", "runner", "trainer", "jacket", "shirt",
  "hoodie", "tote", "slim", "classic", "pro",
];

function seeded(i: number, n: number) {
  return Math.abs((i * 2654435761) % n);
}

export default mutation({
  auth: "guest",
  args: {
    count: v.optional(v.int()),
  },
  async handler(ctx, args) {
    const target = args.count ?? 10_000;

    const existing = await ctx.db.query("Product", {});
    if (existing.length >= target) {
      // Migration for catalogs seeded before slug/featured/salesCount existed:
      // backfill any row missing a slug so the /p/<slug> SSR route + the
      // Featured/Best-selling facets work. Idempotent — skips done rows.
      let backfilled = 0;
      for (const p of existing) {
        const row = p as { id: string; name?: string; slug?: string };
        if (row.slug) continue;
        const name = String(row.name ?? "product");
        const sfx =
          row.id.replace(/[^a-z0-9]/gi, "").slice(-6) ||
          backfilled.toString(16);
        await ctx.db.update("Product", row.id, {
          slug: `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${sfx}`,
          featured: seeded(backfilled + 29, 12) === 0,
          salesCount: seeded(backfilled + 37, 5000),
        });
        backfilled++;
      }
      return { inserted: 0, existing: existing.length, backfilled };
    }

    const start = existing.length;
    const now = new Date().toISOString();
    let inserted = 0;

    for (let i = start; i < target; i++) {
      const brand = BRANDS[seeded(i, BRANDS.length)];
      const category = CATEGORIES[seeded(i + 3, CATEGORIES.length)];
      const color = COLORS[seeded(i + 7, COLORS.length)];
      const adj = ADJECTIVES[seeded(i + 11, ADJECTIVES.length)];
      const noun = NOUNS[seeded(i + 13, NOUNS.length)];

      const name = `${brand} ${adj} ${noun}`;
      const description = `The ${brand} ${name.toLowerCase()} — a ${color} ${category.toLowerCase().slice(0, -1)} designed for everyday wear. ${adj[0].toUpperCase()}${adj.slice(1)} ${noun} construction with a soft feel and long-lasting finish.`;

      // Human-readable slug + a deterministic 4-hex suffix so repeated names
      // across the 10k catalog still map to unique, shareable /p/<slug> URLs.
      const suffix = (seeded(i + 31, 0xffff) | 0x1000).toString(16);
      const slug = `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${suffix}`;

      // Deterministic price + rating from the index so re-seeding
      // produces the same catalog. Prices cluster in the $20-$240
      // range; ratings skew high with natural variance.
      const price = 20 + ((i * 17) % 220) + (i % 100) / 100;
      const rating = 3.2 + ((i * 7) % 180) / 100;
      const stock = seeded(i + 23, 50);
      // ~8% featured; sales skew so a handful are clear best-sellers.
      const featured = seeded(i + 29, 12) === 0;
      const salesCount = seeded(i + 37, 5000);

      await ctx.db.insert("Product", {
        slug,
        name,
        description,
        brand,
        category,
        color,
        price: Math.round(price * 100) / 100,
        rating: Math.round(rating * 10) / 10,
        stock,
        featured,
        salesCount,
        createdAt: now,
      });
      inserted++;
    }

    return { inserted, total: start + inserted };
  },
});
