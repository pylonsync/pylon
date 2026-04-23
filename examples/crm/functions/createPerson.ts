import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: {
    firstName: v.string(),
    lastName: v.optional(v.string()),
    email: v.optional(v.string()),
    phone: v.optional(v.string()),
    title: v.optional(v.string()),
    companyId: v.optional(v.id("Company")),
    ownerId: v.optional(v.id("User")),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");
    if (!args.firstName.trim())
      throw ctx.error("INVALID_NAME", "first name required");

    if (args.companyId) {
      const c = await ctx.db.get("Company", args.companyId);
      if (!c || c.orgId !== ctx.auth.tenantId)
        throw ctx.error("COMPANY_NOT_FOUND", "company not in this org");
    }

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Person", {
      orgId: ctx.auth.tenantId,
      firstName: args.firstName.trim(),
      lastName: args.lastName?.trim() || null,
      email: args.email?.trim().toLowerCase() || null,
      phone: args.phone?.trim() || null,
      title: args.title?.trim() || null,
      companyId: args.companyId || null,
      ownerId: args.ownerId || ctx.auth.userId,
      customFieldsJson: null,
      createdBy: ctx.auth.userId,
      createdAt: now,
      updatedAt: now,
    });
    await ctx.db.insert("Activity", {
      orgId: ctx.auth.tenantId,
      targetType: "Person",
      targetId: id,
      kind: "created",
      metaJson: null,
      actorId: ctx.auth.userId,
      createdAt: now,
    });
    return { personId: id };
  },
});
