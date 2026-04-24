import { mutation, v } from "@pylonsync/functions";

/**
 * Bulk-seed the catalog with ~10k synthetic products. Idempotent —
 * skips if the store is already populated. Runs on first launch of
 * the example so visitors land on a full-looking store page.
 *
 * Performance note: each insert is a separate transaction via
 * `ctx.db.insert`. For a real bulk import you'd wrap the whole thing
 * in a single `ctx.db.transact([...])` call and pay the fsync once.
 * ~10k rows × ~3ms each = 30s on an SSD; fine for a dev seed.
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
  args: {
    count: v.optional(v.int()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const target = args.count ?? 10_000;

    const existing = await ctx.db.query("Product");
    if (existing.length >= target) {
      return { inserted: 0, existing: existing.length };
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

      // Deterministic price + rating from the index so re-seeding
      // produces the same catalog. Prices cluster in the $20-$240
      // range; ratings skew high with natural variance.
      const price = 20 + ((i * 17) % 220) + (i % 100) / 100;
      const rating = 3.2 + ((i * 7) % 180) / 100;
      const stock = seeded(i + 23, 50);

      await ctx.db.insert("Product", {
        name,
        description,
        brand,
        category,
        color,
        price: Math.round(price * 100) / 100,
        rating: Math.round(rating * 10) / 10,
        stock,
        createdAt: now,
      });
      inserted++;
    }

    return { inserted, total: start + inserted };
  },
});
