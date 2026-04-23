import { mutation, v } from "@pylonsync/functions";

export default mutation({
  args: { issueId: v.id("Issue"), body: v.string() },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const body = args.body.trim();
    if (!body) throw ctx.error("EMPTY_COMMENT", "comment body required");

    const issue = await ctx.db.get("Issue", args.issueId);
    if (!issue || issue.orgId !== ctx.auth.tenantId)
      throw ctx.error("ISSUE_NOT_FOUND", "issue not in this org");

    const now = new Date().toISOString();
    const id = await ctx.db.insert("Comment", {
      orgId: ctx.auth.tenantId,
      issueId: args.issueId,
      authorId: ctx.auth.userId,
      body,
      createdAt: now,
      editedAt: null,
    });
    await ctx.db.insert("IssueActivity", {
      orgId: ctx.auth.tenantId,
      issueId: args.issueId,
      actorId: ctx.auth.userId,
      kind: "commented",
      metaJson: JSON.stringify({ commentId: id, preview: body.slice(0, 80) }),
      createdAt: now,
    });
    return { commentId: id };
  },
});
