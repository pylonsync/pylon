import { mutation, v } from "@pylonsync/functions";

// The one User field a person may edit about themselves. Email changes
// need re-verification and passwords have their own route, so the User
// policy denies client writes and this function owns display-name edits.
// Tasks keep a copy of their assignee's name (see the Task entity in
// app.ts), so the person's assigned tasks in their current workspaces are
// updated with the new name.
export default mutation<{ displayName: string }, { displayName: string }>({
  args: { displayName: v.string() },
  async handler(ctx, args) {
    const userId = ctx.auth.userId;
    if (!userId) throw ctx.error("UNAUTHENTICATED", "Sign in first.");
    const displayName = args.displayName.trim();
    if (displayName.length < 1 || displayName.length > 60) {
      throw ctx.error("INVALID_ARGS", "Name must be 1–60 characters.");
    }
    await ctx.db.unsafe.update("User", userId, { displayName });
    // Only in workspaces the person still belongs to: a former member must
    // not be able to write into a workspace they have left.
    const memberships = await ctx.db.unsafe.query("OrgMember", { userId });
    const orgs = new Set(memberships.map((m) => String(m.orgId)));
    const assigned = await ctx.db.unsafe.query("Task", { assigneeId: userId });
    for (const t of assigned) {
      if (orgs.has(String(t.orgId)) && t.assigneeName !== displayName) {
        await ctx.db.unsafe.update("Task", String(t.id), { assigneeName: displayName });
      }
    }
    return { displayName };
  },
});
