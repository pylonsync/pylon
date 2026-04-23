import { mutation, v } from "@pylonsync/functions";

/**
 * Create a product in the active org's catalog. Options are added separately
 * via createProductOption so a configurator can stream them in.
 */
export default mutation({
  args: {
    name: v.string(),
    category: v.string(),
    basePrice: v.number(),
    unit: v.string(),
    sku: v.optional(v.string()),
    description: v.optional(v.string()),
    leadTimeDays: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name is required");
    if (args.basePrice < 0) throw ctx.error("INVALID_PRICE", "price must be ≥ 0");

    // Categories follow Dallas Door Designs' product taxonomy — iron and
    // wood entry doors, barns, pivots, patios, and fiberglass for exterior;
    // interior and architectural cover the indoor lines; "other" is the
    // escape hatch for accessories and services (installation, paint, etc).
    const validCategories = [
      "iron",
      "wood",
      "barn",
      "pivot",
      "patio",
      "fiberglass",
      "interior",
      "architectural",
      "service",
      "other",
    ];
    if (!validCategories.includes(args.category)) {
      throw ctx.error(
        "INVALID_CATEGORY",
        `category must be one of ${validCategories.join(", ")}`,
      );
    }

    const id = await ctx.db.insert("Product", {
      orgId: ctx.auth.tenantId,
      name,
      category: args.category,
      sku: args.sku ?? null,
      description: args.description ?? null,
      basePrice: args.basePrice,
      unit: args.unit,
      active: true,
      leadTimeDays: args.leadTimeDays ?? null,
      createdAt: new Date().toISOString(),
    });
    return { productId: id };
  },
});
