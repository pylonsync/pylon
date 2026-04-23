import { mutation, v } from "@pylonsync/functions";

/**
 * Create a customer record scoped to the caller's active org.
 */
export default mutation({
  args: {
    name: v.string(),
    email: v.optional(v.string()),
    phone: v.optional(v.string()),
    company: v.optional(v.string()),
    addressLine1: v.optional(v.string()),
    addressLine2: v.optional(v.string()),
    city: v.optional(v.string()),
    state: v.optional(v.string()),
    postal: v.optional(v.string()),
    notes: v.optional(v.string()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const name = args.name.trim();
    if (name.length === 0) throw ctx.error("INVALID_NAME", "name is required");

    const id = await ctx.db.insert("Customer", {
      orgId: ctx.auth.tenantId,
      name,
      email: args.email ?? null,
      phone: args.phone ?? null,
      company: args.company ?? null,
      addressLine1: args.addressLine1 ?? null,
      addressLine2: args.addressLine2 ?? null,
      city: args.city ?? null,
      state: args.state ?? null,
      postal: args.postal ?? null,
      notes: args.notes ?? null,
      createdBy: ctx.auth.userId,
      createdAt: new Date().toISOString(),
    });
    return { customerId: id };
  },
});
