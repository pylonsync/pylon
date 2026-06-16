import { mutation } from "@pylonsync/functions";
import { siteConfig } from "../lib/site.config";

// seedProducts — idempotently load the catalog (incl. starting stock) from
// config into the Product table on first visit. The product grid calls this on
// mount; it's a no-op once products exist, so it's safe to call every load. A
// lock keeps two concurrent first-visits from double-seeding.
//
// Public so an anonymous first visitor seeds the shelf — it only ever writes
// the config catalog, never reads or returns anything sensitive.
export default mutation<Record<string, never>, { seeded: boolean; count: number }>({
  auth: "public",
  async handler(ctx) {
    await ctx.db.advisoryLock("shop_seed_products");
    const existing = await ctx.db.unsafe.list("Product");
    if (existing.length > 0) return { seeded: false, count: existing.length };

    for (const p of siteConfig.products.items) {
      await ctx.db.unsafe.insert("Product", {
        slug: p.slug,
        name: p.name,
        priceCents: p.priceCents,
        description: p.description ?? null,
        image: p.image,
        stock: p.stock,
      });
    }
    return { seeded: true, count: siteConfig.products.items.length };
  },
});
