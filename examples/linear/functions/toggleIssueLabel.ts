import { mutation, v } from "@pylonsync/functions";

/**
 * Add or remove a label from an issue. Idempotent — calling with an already-
 * attached label removes it; calling with an unattached label adds it.
 */
export default mutation({
  auth: "guest",
  args: {
    issueId: v.id("Issue"),
    labelId: v.id("Label"),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const issue = await ctx.db.get("Issue", args.issueId as string);
    if (!issue || issue.orgId !== ctx.auth.tenantId)
      throw ctx.error("ISSUE_NOT_FOUND", "issue not in this org");

    const label = await ctx.db.get("Label", args.labelId as string);
    if (!label || label.orgId !== ctx.auth.tenantId)
      throw ctx.error("LABEL_NOT_FOUND", "label not in this org");

    // Check for existing attachment.
    const existing = await ctx.db.query("IssueLabel", {
      issueId: args.issueId,
      labelId: args.labelId,
    });

    const now = new Date().toISOString();
    if (existing.length > 0) {
      // Remove.
      await ctx.db.delete("IssueLabel", existing[0].id as string);
      return { action: "removed" };
    } else {
      // Add.
      await ctx.db.insert("IssueLabel", {
        orgId: ctx.auth.tenantId,
        issueId: args.issueId,
        labelId: args.labelId,
        createdAt: now,
      });
      return { action: "added" };
    }
  },
});
