import { mutation, v } from "@pylonsync/functions";

// reserveCart — hold stock for a whole cart and record its order lines, all in
// ONE transaction. Called only from the `checkout` action (never the public
// HTTP endpoint), so it's `internal: true`: it trusts its args because the
// action already validated them and decided the initial status.
//
// For each line it takes a per-product advisory lock, re-checks stock, and
// DECREMENTS Product.stock — so two carts can't both claim the last unit, and
// every open grid sees "N left" tick down live. A line whose product sold out
// in the meantime is skipped and reported in `soldOut` (partial carts still go
// through). The Order rows it inserts all share `orderGroupId` (= the Stripe
// Checkout Session's client_reference_id) so the webhook can settle them as one.
//
// `initialStatus` is set by the caller: "pending" when a Stripe payment is
// coming, "reserved" when there's no payment processor and the owner follows up.
export default mutation<
  {
    orderGroupId: string;
    initialStatus: "pending" | "reserved";
    customerName: string;
    customerEmail: string;
    items: { slug: string; qty: number }[];
  },
  {
    ok: boolean;
    lines: { slug: string; name: string; qty: number; unitPriceCents: number }[];
    soldOut: string[];
  }
>({
  internal: true,
  args: {
    orderGroupId: v.string(),
    initialStatus: v.string(),
    customerName: v.string(),
    customerEmail: v.string(),
    items: v.array(
      v.object({
        slug: v.string(),
        qty: v.int(),
      }),
    ),
  },
  async handler(ctx, args) {
    const name = args.customerName.trim();
    const email = args.customerEmail.trim().toLowerCase();
    const lines: { slug: string; name: string; qty: number; unitPriceCents: number }[] = [];
    const soldOut: string[] = [];

    for (const item of args.items) {
      const qty = Math.trunc(item.qty);
      if (!Number.isFinite(qty) || qty < 1) continue;

      // Serialize against other orders for this product so the re-check can't
      // race; held until the tx commits.
      await ctx.db.advisoryLock(`shop_product:${item.slug}`);

      const product = (await ctx.db.unsafe.lookup("Product", "slug", item.slug)) as
        | { id: string; name: string; priceCents: number; stock: number }
        | null;
      if (!product) continue;
      if (product.stock <= 0) {
        soldOut.push(product.name);
        continue;
      }

      // Clamp to what's actually on the shelf; if some are left, take those.
      const take = Math.min(qty, product.stock);
      await ctx.db.unsafe.update("Product", product.id, { stock: product.stock - take });
      await ctx.db.unsafe.insert("Order", {
        orderGroupId: args.orderGroupId,
        productSlug: item.slug,
        productName: product.name,
        qty: take,
        unitPriceCents: product.priceCents,
        customerName: name,
        customerEmail: email,
        status: args.initialStatus,
        createdAt: new Date().toISOString(),
      });
      lines.push({ slug: item.slug, name: product.name, qty: take, unitPriceCents: product.priceCents });
      if (take < qty) soldOut.push(product.name);
    }

    return { ok: lines.length > 0, lines, soldOut };
  },
});
