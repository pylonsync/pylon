import { mutation, v } from "@pylonsync/functions";

// releaseGroup — return a cart's held stock to the shelf and mark its lines
// "cancelled". Called when a Stripe Checkout Session expires or its payment
// fails (via the webhook), or when checkout itself couldn't open the session.
// Internal + idempotent: it only acts on rows still "pending" (stock still
// held), so it never double-restores. The Product.stock bump syncs to every
// open grid, so "Sold out" can flip back to "N left" live.
export default mutation<{ orderGroupId: string }, { ok: boolean; restored: number }>({
  internal: true,
  args: { orderGroupId: v.string() },
  async handler(ctx, args) {
    const rows = (await ctx.db.unsafe.list("Order")) as unknown as {
      id: string;
      orderGroupId: string;
      productSlug: string;
      qty: number;
      status: string;
    }[];
    let restored = 0;
    for (const o of rows) {
      if (o.orderGroupId !== args.orderGroupId || o.status !== "pending") continue;

      await ctx.db.advisoryLock(`shop_product:${o.productSlug}`);
      const product = (await ctx.db.unsafe.lookup("Product", "slug", o.productSlug)) as
        | { id: string; stock: number }
        | null;
      if (product) {
        await ctx.db.unsafe.update("Product", product.id, { stock: product.stock + o.qty });
      }
      await ctx.db.unsafe.update("Order", o.id, { status: "cancelled" });
      restored += o.qty;
    }
    return { ok: true, restored };
  },
});
