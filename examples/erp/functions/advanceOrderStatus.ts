import { mutation, v } from "@pylonsync/functions";

/**
 * Advance an order through the production pipeline. Status is a small
 * state machine: confirmed → in_production → ready → shipped → delivered.
 * Cancel is allowed from any non-terminal state. Backwards moves are
 * rejected so a stray click can't un-ship an order.
 */
const ORDER = ["confirmed", "in_production", "ready", "shipped", "delivered"];

export default mutation({
  args: {
    orderId: v.id("Order"),
    status: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const order = await ctx.db.get("Order", args.orderId);
    if (!order || order.orgId !== ctx.auth.tenantId) {
      throw ctx.error("ORDER_NOT_FOUND", "order does not belong to this org");
    }
    if (order.status === "cancelled" || order.status === "delivered") {
      throw ctx.error(
        "TERMINAL_STATUS",
        `order is ${order.status} and cannot be updated`,
      );
    }

    const now = new Date().toISOString();
    const patch: Record<string, unknown> = { status: args.status };

    if (args.status === "cancelled") {
      patch.cancelledAt = now;
    } else {
      const curIdx = ORDER.indexOf(order.status);
      const nextIdx = ORDER.indexOf(args.status);
      if (nextIdx === -1) {
        throw ctx.error("INVALID_STATUS", `unknown status "${args.status}"`);
      }
      if (nextIdx <= curIdx) {
        throw ctx.error(
          "CANNOT_REGRESS",
          `cannot move from ${order.status} back to ${args.status}`,
        );
      }
      if (args.status === "shipped") patch.shippedAt = now;
      if (args.status === "delivered") patch.deliveredAt = now;
    }

    await ctx.db.update("Order", args.orderId, patch);
    return { orderId: args.orderId, status: args.status };
  },
});
