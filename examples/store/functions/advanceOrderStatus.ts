import { mutation, v } from "@pylonsync/functions";

/**
 * Internal: bump an order's status. Invoked by the scheduler after
 * `placeOrder` enqueues the timeline. Skips if the order has already
 * advanced past this state, so re-deliveries from the scheduler are
 * idempotent.
 */
const ORDER: Record<string, number> = {
  placed: 0,
  packed: 1,
  shipped: 2,
  delivered: 3,
};

export default mutation({
  args: {
    orderId: v.string(),
    to: v.string(),
  },
  async handler(ctx, args) {
    const order = (await ctx.db.get("Order", args.orderId)) as
      | { status: string }
      | null;
    if (!order) return { advanced: false, reason: "not_found" };

    const current = ORDER[order.status] ?? -1;
    const next = ORDER[args.to] ?? -1;
    if (next <= current) {
      return { advanced: false, reason: "already_advanced" };
    }

    await ctx.db.update("Order", args.orderId, { status: args.to });
    return { advanced: true, status: args.to };
  },
});
