import { mutation, v } from "@pylonsync/functions";

/**
 * Atomically convert the user's cart into an order.
 *
 *   1. Read current cart items
 *   2. Create the Order with a snapshot of the shipping address
 *   3. Create one OrderItem per cart row
 *   4. Clear the cart
 *   5. Schedule status progression (placed → packed → shipped → delivered)
 *
 * Steps 1-4 run in a single `ctx.db` mutation transaction — if any
 * step throws, nothing commits. The scheduled progression in step 5
 * is enqueued through `ctx.scheduler.runAfter` and is durable: even
 * if the server restarts, the scheduler picks it back up.
 */
export default mutation({
  args: {
    addressId: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const userId = ctx.auth.userId;

    const address = await ctx.db.get("Address", args.addressId);
    if (!address || address.userId !== userId) {
      throw ctx.error("ADDRESS_NOT_FOUND", "shipping address not found");
    }

    const cart = await ctx.db.query("CartItem");
    const mine = cart.filter((c) => (c as { userId: string }).userId === userId);
    if (mine.length === 0) {
      throw ctx.error("EMPTY_CART", "cart is empty");
    }

    const subtotal = mine.reduce(
      (s, c) =>
        s +
        (c as { productPrice: number; quantity: number }).productPrice *
          (c as { quantity: number }).quantity,
      0,
    );
    const itemCount = mine.reduce(
      (n, c) => n + (c as { quantity: number }).quantity,
      0,
    );
    const placedAt = new Date();
    // Demo timeline: ~90 seconds end-to-end. Replace with a realistic
    // estimate (e.g. placedAt + 3 days) in production.
    const eta = new Date(placedAt.getTime() + 90_000);
    const tracking = `PYL${Math.floor(Math.random() * 1e9)
      .toString(36)
      .toUpperCase()
      .padStart(7, "0")}`;

    const orderId = await ctx.db.insert("Order", {
      userId,
      status: "placed",
      subtotal: Math.round(subtotal * 100) / 100,
      itemCount,
      shipName: address.fullName,
      shipStreet: address.street,
      shipCity: address.city,
      shipPostal: address.postal,
      shipCountry: address.country,
      placedAt: placedAt.toISOString(),
      trackingNumber: tracking,
      estimatedDelivery: eta.toISOString(),
    });

    for (const c of mine as Array<{
      productId: string;
      productName: string;
      productBrand: string;
      productPrice: number;
      quantity: number;
    }>) {
      await ctx.db.insert("OrderItem", {
        orderId,
        userId,
        productId: c.productId,
        productName: c.productName,
        productBrand: c.productBrand,
        unitPrice: c.productPrice,
        quantity: c.quantity,
      });
    }

    for (const c of mine as Array<{ id: string }>) {
      await ctx.db.delete("CartItem", c.id);
    }

    // Status progression. Times are short for the demo so users
    // actually see the timeline animate; bump these to hours/days
    // for a real shop.
    await ctx.scheduler.runAfter(15_000, "advanceOrderStatus", {
      orderId,
      to: "packed",
    });
    await ctx.scheduler.runAfter(30_000, "advanceOrderStatus", {
      orderId,
      to: "shipped",
    });
    await ctx.scheduler.runAfter(90_000, "advanceOrderStatus", {
      orderId,
      to: "delivered",
    });

    return { orderId, subtotal, itemCount, trackingNumber: tracking };
  },
});
