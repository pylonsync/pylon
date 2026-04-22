import { mutation, v } from "@statecraft/functions";

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

    const validCategories = ["door", "window", "cabinet", "hardware", "other"];
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
