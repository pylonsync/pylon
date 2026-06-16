import { mutation, v } from "@pylonsync/functions";
import { emailMatchesOwner } from "../lib/owner";

// cancelOrder — owner-only. Marks the order cancelled AND returns its units to
// stock: the Product.stock bump syncs to every open grid, so "N left" ticks
// back up (and a sold-out item can come back) live.
export default mutation<{ orderId: string }, { ok: boolean }>({
  auth: "user",
  args: { orderId: v.id("Order") },
  async handler(ctx, args) {
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!emailMatchesOwner(me?.email as string | undefined, ctx.env.PYLON_OWNER_EMAIL)) {
      throw ctx.error("POLICY_DENIED", "Only the owner can manage orders.");
    }
    const order = (await ctx.db.get("Order", args.orderId)) as
      | { productSlug: string; qty: number; status: string }
      | null;
    if (!order) throw ctx.error("NOT_FOUND", "Order not found.");

    await ctx.db.unsafe.update("Order", args.orderId, { status: "cancelled" });

    // Restore the units (only if it wasn't already cancelled).
    if (order.status !== "cancelled") {
      const product = (await ctx.db.unsafe.lookup("Product", "slug", order.productSlug)) as
        | { id: string; stock: number }
        | null;
      if (product) {
        await ctx.db.unsafe.update("Product", product.id, { stock: product.stock + order.qty });
      }
    }
    return { ok: true };
  },
});
