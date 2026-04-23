import { mutation, v } from "@pylonsync/functions";

/**
 * Add a material to inventory. Stock goes in empty by default; use
 * adjustStock to record receipts.
 */
export default mutation({
  args: {
    name: v.string(),
    unit: v.string(),
    costPerUnit: v.number(),
    reorderPoint: v.optional(v.number()),
    sku: v.optional(v.string()),
    supplier: v.optional(v.string()),
    notes: v.optional(v.string()),
    initialStock: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name is required");
    if (args.costPerUnit < 0) throw ctx.error("INVALID_COST", "cost must be ≥ 0");
    const initial = args.initialStock ?? 0;
    if (initial < 0) throw ctx.error("INVALID_STOCK", "stock must be ≥ 0");

    const now = new Date().toISOString();
    const materialId = await ctx.db.insert("Material", {
      orgId: ctx.auth.tenantId,
      name,
      sku: args.sku ?? null,
      unit: args.unit,
      stockQty: initial,
      reorderPoint: args.reorderPoint ?? 0,
      costPerUnit: args.costPerUnit,
      supplier: args.supplier ?? null,
      notes: args.notes ?? null,
      createdAt: now,
    });

    if (initial > 0) {
      await ctx.db.insert("StockMovement", {
        orgId: ctx.auth.tenantId,
        materialId,
        delta: initial,
        reason: "receipt",
        reference: "initial stock",
        performedBy: ctx.auth.userId,
        createdAt: now,
      });
    }

    return { materialId };
  },
});
