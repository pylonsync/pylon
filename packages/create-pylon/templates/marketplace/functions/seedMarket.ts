import { mutation, v } from "@pylonsync/functions";

// A handful of believable listings so the marketplace isn't empty on first
// run. Idempotent: seeds only when the catalog is empty, so the browse page
// can call it on load without worrying about duplicates.
const DEMO: Array<{
  seller: string;
  title: string;
  description: string;
  price: number;
  category: string;
  condition: string;
  seed: string;
}> = [
  { seller: "maple-fox", title: "Herman Miller Aeron (size B)", description: "Fully loaded, posture-fit SL. Light desk use, no squeaks.", price: 540, category: "furniture", condition: "like-new", seed: "a1f3" },
  { seller: "amber-lynx", title: "Kodak Retina IIa rangefinder", description: "1950s folding 35mm. Clean glass, accurate shutter, leather case.", price: 185, category: "cameras", condition: "good", seed: "b7c2" },
  { seller: "swift-otter", title: "Specialized Allez road bike, 54cm", description: "Shimano Claris, fresh bar tape, recently tuned. Fast commuter.", price: 420, category: "bikes", condition: "good", seed: "c4e9" },
  { seller: "cosmic-wren", title: "Vintage Technics SL-1200 MK2", description: "Legendary direct-drive turntable. Spins true, new slipmat.", price: 650, category: "audio", condition: "good", seed: "d8a1" },
  { seller: "ivory-sparrow", title: "Le Creuset 5.5qt Dutch oven", description: "Flame orange, enamel intact. The one everyone wants.", price: 120, category: "kitchen", condition: "like-new", seed: "e2b6" },
  { seller: "slate-heron", title: 'LG 27" 4K UltraFine monitor', description: "USB-C, 60Hz, color-accurate. Box + cables included.", price: 240, category: "electronics", condition: "good", seed: "f5d0" },
  { seller: "maple-fox", title: "Eames-style lounge + ottoman", description: "Walnut + tan leather replica. Sturdy, very comfortable.", price: 380, category: "furniture", condition: "good", seed: "a9c4" },
  { seller: "amber-lynx", title: "Patagonia Black Hole 55L duffel", description: "One trip old. Weatherproof, zips perfect. Forest green.", price: 75, category: "outdoor", condition: "like-new", seed: "b3f8" },
  { seller: "cosmic-wren", title: "Fender Player Stratocaster, sunburst", description: "Maple neck, plays great. Small buckle rash on back.", price: 560, category: "instruments", condition: "good", seed: "d1e7" },
  { seller: "slate-heron", title: "iPad Air (5th gen) 64GB", description: "Space gray, 100% battery health, screen flawless.", price: 330, category: "electronics", condition: "like-new", seed: "f0a2" },
  { seller: "ivory-sparrow", title: "Mid-century teak sideboard", description: "Three drawers, two cabinets. Real teak, refinished top.", price: 295, category: "furniture", condition: "good", seed: "e7c3" },
  { seller: "swift-otter", title: "Brooks Brothers wool overcoat, 40R", description: "Charcoal herringbone, fully lined. Dry-cleaned, ready to wear.", price: 90, category: "apparel", condition: "good", seed: "c2b5" },
];

interface SeedMarketArgs {
  start?: number;
  end?: number;
}

interface SeedMarketResult {
  seeded: number;
}

export default mutation<SeedMarketArgs, SeedMarketResult>({
  // Defaults to auth: "user" — seeds a slice of the catalog owned by the
  // caller. The bootstrap calls this twice: once as a "bazaar" seller for the
  // bulk of the catalog (so the demo buyer can bid on it), and once as the
  // demo account for a couple of its own listings.
  args: {
    start: v.optional(v.number()),
    end: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "sign in first");

    // Per-caller idempotency: skip if THIS seller already has listings, so the
    // two seed calls (and any reloads) don't duplicate.
    const all = await ctx.db.list("Listing") as Array<{ sellerId: string }>;
    if (all.some((l) => l.sellerId === ctx.auth.userId)) return { seeded: 0 };

    const start = args.start ?? 0;
    const end = args.end ?? DEMO.length;
    const slice = DEMO.slice(start, end);

    // Stagger createdAt so the grid + ticker have a believable order. The
    // seller id is the caller — `Listing.sellerId` is `field.owner()`, so the
    // framework would reject any other value. `seller` stays a display name.
    const slugify = (s: string) =>
      s
        .toLowerCase()
        .trim()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 60);

    const now = Date.now();
    let n = 0;
    for (const d of slice) {
      await ctx.db.insert("Listing", {
        sellerId: ctx.auth.userId,
        sellerName: d.seller,
        title: d.title,
        slug: `${slugify(d.title) || "item"}-${d.seed}`,
        description: d.description,
        price: d.price,
        category: d.category,
        condition: d.condition,
        status: "active",
        seed: d.seed,
        createdAt: new Date(now - (start + n) * 7 * 60_000).toISOString(),
      });
      n++;
    }
    return { seeded: n };
  },
});
