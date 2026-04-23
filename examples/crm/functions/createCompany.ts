import { mutation, v } from "@pylonsync/functions";

/** Create a company scoped to the caller's active org. Logs an Activity
 *  row so the timeline picks it up without the client having to juggle
 *  two mutations. */
export default mutation({
  args: {
    name: v.string(),
    domain: v.optional(v.string()),
    industry: v.optional(v.string()),
    sizeBucket: v.optional(v.string()),
    status: v.optional(v.string()),
    description: v.optional(v.string()),
    ownerId: v.optional(v.id("User")),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");
    if (!args.name.trim()) throw ctx.error("INVALID_NAME", "name required");

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Company", {
      orgId: ctx.auth.tenantId,
      name: args.name.trim(),
      domain: args.domain?.trim() || null,
      industry: args.industry || null,
      sizeBucket: args.sizeBucket || null,
      status: args.status || "lead",
      description: args.description?.trim() || null,
      ownerId: args.ownerId || ctx.auth.userId,
      customFieldsJson: null,
      createdBy: ctx.auth.userId,
      createdAt: now,
      updatedAt: now,
    });
    await ctx.db.insert("Activity", {
      orgId: ctx.auth.tenantId,
      targetType: "Company",
      targetId: id,
      kind: "created",
      metaJson: null,
      actorId: ctx.auth.userId,
      createdAt: now,
    });
    return { companyId: id };
  },
});
