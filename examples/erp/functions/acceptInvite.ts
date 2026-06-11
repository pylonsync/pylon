import { mutation, v } from "@pylonsync/functions";

/**
 * Accept an outstanding invite. Takes the invite id (client resolves it by
 * looking up an OrgInvite row with the user's email). Creates the OrgMember
 * row and stamps the invite's acceptedAt.
 */
export default mutation<{ inviteId: string }, { orgId: string; alreadyMember: boolean }>({
  auth: "guest",
  args: {
    inviteId: v.id("OrgInvite"),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");

    const invite = await ctx.db.get("OrgInvite", args.inviteId);
    if (!invite) throw ctx.error("INVITE_NOT_FOUND", "invite does not exist");
    if ((invite as { acceptedAt?: string | null }).acceptedAt) {
      throw ctx.error("ALREADY_ACCEPTED", "this invite was already accepted");
    }

    // The invite's email must match the caller's User email — this keeps
    // a valid invite from being claimed by a different user just because
    // they saw the id somehow.
    const me = await ctx.db.get("User", ctx.auth.userId);
    const inviteEmail = (invite as { email: string }).email;
    const inviteOrgId = (invite as { orgId: string }).orgId;
    const inviteRole = (invite as { role: string }).role;
    const inviteInvitedBy = (invite as { invitedBy: string | null }).invitedBy;
    if (!me || (me as { email: string }).email.toLowerCase() !== inviteEmail.toLowerCase()) {
      throw ctx.error(
        "EMAIL_MISMATCH",
        "this invite is for a different email address",
      );
    }

    // Already a member? Just consume the invite.
    const existing = await ctx.db.query("OrgMember", {
      userId: ctx.auth.userId,
      orgId: inviteOrgId,
    });
    if (existing.length > 0) {
      await ctx.db.update("OrgInvite", args.inviteId, {
        acceptedAt: new Date().toISOString(),
      });
      return { orgId: inviteOrgId, alreadyMember: true };
    }

    const now = new Date().toISOString();
    await ctx.db.insert("OrgMember", {
      userId: ctx.auth.userId,
      orgId: inviteOrgId,
      role: inviteRole,
      invitedBy: inviteInvitedBy,
      joinedAt: now,
    });
    await ctx.db.update("OrgInvite", args.inviteId, { acceptedAt: now });

    return { orgId: inviteOrgId, alreadyMember: false };
  },
});
