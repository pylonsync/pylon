import { mutation, v } from "@pylonsync/functions";

/**
 * Change a member's role in the active org. Only owners can set the owner
 * role or downgrade another owner; admins can manage estimators/production/
 * viewer/admin but can't touch owners. Users cannot edit their own role.
 */
export default mutation({
  args: {
    memberId: v.id("OrgMember"),
    role: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const validRoles = ["owner", "admin", "estimator", "production", "viewer"];
    if (!validRoles.includes(args.role)) {
      throw ctx.error(
        "INVALID_ROLE",
        `role must be one of ${validRoles.join(", ")}`,
      );
    }

    const target = await ctx.db.get("OrgMember", args.memberId);
    if (!target) throw ctx.error("NOT_FOUND", "member not found");
    if (target.orgId !== ctx.auth.tenantId) {
      throw ctx.error("FORBIDDEN", "member belongs to another org");
    }

    if (target.userId === ctx.auth.userId) {
      throw ctx.error("SELF_EDIT", "you cannot edit your own role");
    }

    const myMemberships = await ctx.db.query("OrgMember", {
      userId: ctx.auth.userId,
      orgId: ctx.auth.tenantId,
    });
    const me = myMemberships[0];
    if (!me) throw ctx.error("FORBIDDEN", "you are not a member of this org");

    if (me.role === "owner") {
      // Owner can do anything.
    } else if (me.role === "admin") {
      if (target.role === "owner" || args.role === "owner") {
        throw ctx.error("FORBIDDEN", "only owners can manage owner roles");
      }
    } else {
      throw ctx.error("FORBIDDEN", "only owners/admins can change roles");
    }

    await ctx.db.update("OrgMember", args.memberId, { role: args.role });
    return { memberId: args.memberId, role: args.role };
  },
});
