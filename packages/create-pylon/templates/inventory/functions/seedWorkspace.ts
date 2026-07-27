import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Fill a brand-new stock workspace once.
 *
 * Seeds a plausible SEQUENCE of receipts and sales rather than a starting
 * quantity, because on-hand here is the sum of the ledger — a seed that wrote
 * levels directly would contradict the app\'s own model.
 *
 * Returns immediately if any product exists, so it is safe on every load.
 * Delete this function and lib/seed.ts once you stock real products.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    const existing = await ctx.db.query("Product", { $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    const productIds = new Map<string, string>();
    for (const product of seed.products) {
      const id = await ctx.db.insert("Product", product.row);
      productIds.set(product.key, id as string);
    }

    for (const movement of seed.movements) {
      await ctx.db.insert("Movement", {
        ...movement.row,
        productId: productIds.get(movement.product) ?? null,
        actorId: me,
      });
    }

    return { seeded: true };
  },
});
