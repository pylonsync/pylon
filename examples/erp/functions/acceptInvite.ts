import { mutation, v } from "@statecraft/functions";

/**
 * Accept an outstanding invite. Takes the invite id (client resolves it by
 * looking up an OrgInvite row with the user's email). Creates the OrgMember
 * row and stamps the invite's acceptedAt.
 */
export default mutation({
  args: {
    inviteId: v.id("OrgInvite"),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const invite = await ctx.db.get("OrgInvite", args.inviteId);
    if (!invite) throw ctx.error("INVITE_NOT_FOUND", "invite does not exist");
    if (invite.acceptedAt) {
      throw ctx.error("ALREADY_ACCEPTED", "this invite was already accepted");
    }

    // The invite's email must match the caller's User email — this keeps
    // a valid invite from being claimed by a different user just because
    // they saw the id somehow.
    const me = await ctx.db.get("User", ctx.auth.userId);
    if (!me || me.email.toLowerCase() !== invite.email.toLowerCase()) {
      throw ctx.error(
        "EMAIL_MISMATCH",
        "this invite is for a different email address",
      );
    }

    // Already a member? Just consume the invite.
    const existing = await ctx.db.query("OrgMember", {
      userId: ctx.auth.userId,
      orgId: invite.orgId,
    });
    if (existing.length > 0) {
      await ctx.db.update("OrgInvite", invite.id, {
        acceptedAt: new Date().toISOString(),
      });
      return { orgId: invite.orgId, alreadyMember: true };
    }

    const now = new Date().toISOString();
    await ctx.db.insert("OrgMember", {
      userId: ctx.auth.userId,
      orgId: invite.orgId,
      role: invite.role,
      invitedBy: invite.invitedBy,
      joinedAt: now,
    });
    await ctx.db.update("OrgInvite", invite.id, { acceptedAt: now });

    return { orgId: invite.orgId, alreadyMember: false };
  },
});
