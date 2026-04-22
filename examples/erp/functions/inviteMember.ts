import { mutation, v } from "@statecraft/functions";

/**
 * Invite someone to the active org. Only owners/admins can invite. Email
 * uniqueness is per-org (so reinvites bump the same row).
 *
 * This demo does NOT send email — the invite row is the source of truth
 * and the invited user sees it on next login via an "Invites" banner.
 */
export default mutation({
  args: {
    email: v.string(),
    role: v.string(),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) {
      throw ctx.error("NO_ACTIVE_ORG", "select an organization first");
    }

    // Verify the caller's role in the active org.
    const myMemberships = await ctx.db.query("OrgMember", {
      userId: ctx.auth.userId,
      orgId: ctx.auth.tenantId,
    });
    if (myMemberships.length === 0) {
      throw ctx.error("FORBIDDEN", "you are not a member of this org");
    }
    const role = myMemberships[0].role;
    if (role !== "owner" && role !== "admin") {
      throw ctx.error("FORBIDDEN", "only owners/admins can invite members");
    }

    const email = args.email.trim().toLowerCase();
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      throw ctx.error("INVALID_EMAIL", "invalid email address");
    }
    const validRoles = ["admin", "estimator", "production", "viewer"];
    if (!validRoles.includes(args.role)) {
      throw ctx.error(
        "INVALID_ROLE",
        `role must be one of ${validRoles.join(", ")}`,
      );
    }

    // If an active invite already exists, update it (reinvite = change role).
    const existing = await ctx.db.query("OrgInvite", {
      orgId: ctx.auth.tenantId,
      email,
    });
    const pending = existing.find((i) => !i.acceptedAt);
    if (pending) {
      await ctx.db.update("OrgInvite", pending.id, {
        role: args.role,
        invitedBy: ctx.auth.userId,
      });
      return { inviteId: pending.id, reinvited: true };
    }

    const id = await ctx.db.insert("OrgInvite", {
      orgId: ctx.auth.tenantId,
      email,
      role: args.role,
      invitedBy: ctx.auth.userId,
      createdAt: new Date().toISOString(),
      acceptedAt: null,
    });
    return { inviteId: id, reinvited: false };
  },
});
