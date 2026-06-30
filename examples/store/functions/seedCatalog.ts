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

// --- Derived facet/display fields ---------------------------------------
// Faceted search filters on equality, not ranges, so price + rating are
// bucketed into string tiers that show as sidebar facets.
function priceBucket(price: number): string {
  if (price < 25) return "Under $25";
  if (price < 50) return "$25 – $50";
  if (price < 100) return "$50 – $100";
  if (price < 200) return "$100 – $200";
  return "$200 & up";
}

function ratingTier(rating: number): string {
  if (rating >= 4.5) return "4.5★ & up";
  if (rating >= 4) return "4 – 4.5★";
  if (rating >= 3.5) return "3.5 – 4★";
  return "Under 3.5★";
}

// Sizes only apply to apparel; accessories/home are one-size. Stored as a
// comma-joined string (no array field type) — the client splits it.
function sizesFor(category: string): string {
  switch (category) {
    case "Shoes":
      return "7,8,9,10,11,12";
    case "Pants":
      return "28,30,32,34,36";
    case "Shirts":
    case "Jackets":
      return "XS,S,M,L,XL";
    case "Hats":
      return "S/M,L/XL";
    default:
      return "";
  }
}

// A few merchandising tags on *some* items (comma-joined). Deterministic so
// re-seeds are stable; most items get none, some get one or two.
function tagsFor(i: number): string {
  const t: string[] = [];
  if (i % 4 === 0) t.push("Sale");
  if (i % 6 === 0) t.push("New");
  if (i % 11 === 0) t.push("Eco");
  if (i % 9 === 0) t.push("Limited");
  return t.join(",");
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
      // Migration for catalogs seeded before the slug/facet/display fields
      // existed. Idempotent: a row already carrying a priceBucket is done.
      // Backfills slug too (older rows) without changing an existing one.
      let backfilled = 0;
      let i = 0;
      for (const p of existing) {
        const row = p as {
          id: string;
          name?: string;
          slug?: string;
          price?: number;
          rating?: number;
          category?: string;
          priceBucket?: string;
        };
        if (row.priceBucket) {
          i++;
          continue;
        }
        const name = String(row.name ?? "product");
        const price = Number(row.price ?? 0);
        const rating = Number(row.rating ?? 0);
        const category = String(row.category ?? "");
        const patch: Record<string, unknown> = {
          priceBucket: priceBucket(price),
          ratingTier: ratingTier(rating),
          sizes: sizesFor(category),
          tags: tagsFor(i),
        };
        if (!row.slug) {
          const sfx =
            row.id.replace(/[^a-z0-9]/gi, "").slice(-6) || i.toString(16);
          patch.slug = `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${sfx}`;
          patch.featured = seeded(i + 29, 12) === 0;
          patch.salesCount = seeded(i + 37, 5000);
        }
        await ctx.db.update("Product", row.id, patch);
        backfilled++;
        i++;
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

      // Human-readable slug + a deterministic hex suffix so repeated names
      // across the 10k catalog still map to unique, shareable /p/<slug> URLs.
      const suffix = (seeded(i + 31, 0xffff) | 0x1000).toString(16);
      const slug = `${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}-${suffix}`;

      // Deterministic price + rating from the index so re-seeding
      // produces the same catalog. Prices cluster in the $20-$240
      // range; ratings skew high with natural variance.
      const price = Math.round((20 + ((i * 17) % 220) + (i % 100) / 100) * 100) / 100;
      const rating = Math.round((3.2 + ((i * 7) % 180) / 100) * 10) / 10;
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
        price,
        rating,
        stock,
        featured,
        salesCount,
        priceBucket: priceBucket(price),
        ratingTier: ratingTier(rating),
        sizes: sizesFor(category),
        tags: tagsFor(i),
        createdAt: now,
      });
      inserted++;
    }

    return { inserted, total: start + inserted };
  },
});
