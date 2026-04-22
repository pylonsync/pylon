import { mutation, v } from "@statecraft/functions";

/**
 * Create an order with line items in one call. Totals are computed
 * server-side from the line inputs so a client can't ship a 3 for "$1".
 * Each line must reference a product in the same org.
 *
 * Line input: { productId, description, configJson?, qty, unitPrice }
 * lineTotal is computed; productionStatus starts as "queued".
 */
export default mutation({
  args: {
    customerId: v.id("Customer"),
    notes: v.optional(v.string()),
    dueDate: v.optional(v.string()),
    taxRate: v.optional(v.number()),
    lines: v.array(
      v.object({
        productId: v.id("Product"),
        description: v.string(),
        configJson: v.optional(v.string()),
        qty: v.number(),
        unitPrice: v.number(),
      }),
    ),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");
    if (args.lines.length === 0) {
      throw ctx.error("EMPTY_ORDER", "order must have at least one line");
    }

    const customer = await ctx.db.get("Customer", args.customerId);
    if (!customer || customer.orgId !== ctx.auth.tenantId) {
      throw ctx.error("CUSTOMER_NOT_FOUND", "customer does not belong to this org");
    }

    // Validate every product up front so we don't half-write an order.
    for (const line of args.lines) {
      if (line.qty <= 0) throw ctx.error("INVALID_QTY", "qty must be > 0");
      if (line.unitPrice < 0) throw ctx.error("INVALID_PRICE", "price must be ≥ 0");
      const product = await ctx.db.get("Product", line.productId);
      if (!product || product.orgId !== ctx.auth.tenantId) {
        throw ctx.error(
          "PRODUCT_NOT_FOUND",
          `product ${line.productId} does not belong to this org`,
        );
      }
    }

    let subtotal = 0;
    for (const line of args.lines) {
      subtotal += line.qty * line.unitPrice;
    }
    const taxRate = args.taxRate ?? 0;
    const tax = Math.round(subtotal * taxRate * 100) / 100;
    const total = subtotal + tax;

    // Simple per-org order number: count existing orders + 1. Real systems
    // want a dedicated sequence so concurrent creates don't collide, but
    // this keeps the demo legible.
    const existing = await ctx.db.query("Order", {
      orgId: ctx.auth.tenantId,
    });
    const year = new Date().getFullYear();
    const number = `SO-${year}-${String(existing.length + 1).padStart(4, "0")}`;
    const now = new Date().toISOString();

    const orderId = await ctx.db.insert("Order", {
      orgId: ctx.auth.tenantId,
      customerId: args.customerId,
      quoteId: null,
      number,
      status: "confirmed",
      subtotal: Math.round(subtotal * 100) / 100,
      tax,
      total: Math.round(total * 100) / 100,
      notes: args.notes ?? null,
      dueDate: args.dueDate ?? null,
      shippedAt: null,
      deliveredAt: null,
      cancelledAt: null,
      createdBy: ctx.auth.userId,
      createdAt: now,
    });

    for (let i = 0; i < args.lines.length; i++) {
      const line = args.lines[i];
      const lineTotal = Math.round(line.qty * line.unitPrice * 100) / 100;
      await ctx.db.insert("OrderLine", {
        orgId: ctx.auth.tenantId,
        orderId,
        productId: line.productId,
        description: line.description,
        configJson: line.configJson ?? null,
        qty: line.qty,
        unitPrice: line.unitPrice,
        lineTotal,
        productionStatus: "queued",
        sortOrder: i,
      });
    }

    return { orderId, number };
  },
});
