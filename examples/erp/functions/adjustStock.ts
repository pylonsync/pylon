import { mutation, v } from "@pylonsync/functions";

/**
 * Record a stock movement and update the material's on-hand qty. Delta is
 * signed — positive for receipts, negative for issues/waste.
 *
 * Writes Material + StockMovement in the same mutation so the ledger
 * never drifts from the cached qty. If stock would go negative, we error
 * by default; pass `allowNegative: true` to explicitly accept it (e.g.
 * reconciling an undercount).
 */
export default mutation({
  args: {
    materialId: v.id("Material"),
    delta: v.number(),
    reason: v.string(),
    reference: v.optional(v.string()),
    allowNegative: v.optional(v.boolean()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const validReasons = ["receipt", "issue", "adjust", "waste"];
    if (!validReasons.includes(args.reason)) {
      throw ctx.error(
        "INVALID_REASON",
        `reason must be one of ${validReasons.join(", ")}`,
      );
    }
    if (args.delta === 0) {
      throw ctx.error("ZERO_DELTA", "delta must be non-zero");
    }

    const material = await ctx.db.get("Material", args.materialId);
    if (!material) throw ctx.error("NOT_FOUND", "material not found");
    if (material.orgId !== ctx.auth.tenantId) {
      throw ctx.error("FORBIDDEN", "material belongs to another org");
    }

    const nextQty = material.stockQty + args.delta;
    if (nextQty < 0 && !args.allowNegative) {
      throw ctx.error(
        "INSUFFICIENT_STOCK",
        `only ${material.stockQty} ${material.unit} on hand`,
      );
    }

    const now = new Date().toISOString();
    await ctx.db.update("Material", args.materialId, { stockQty: nextQty });
    await ctx.db.insert("StockMovement", {
      orgId: ctx.auth.tenantId,
      materialId: args.materialId,
      delta: args.delta,
      reason: args.reason,
      reference: args.reference ?? null,
      performedBy: ctx.auth.userId,
      createdAt: now,
    });

    return { materialId: args.materialId, stockQty: nextQty };
  },
});
