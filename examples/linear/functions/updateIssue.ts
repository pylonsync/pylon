import { mutation, v } from "@pylonsync/functions";

/**
 * Patch an issue. All fields optional; server records an IssueActivity row
 * for each change so the timeline reflects what moved. State transitions
 * stamp startedAt/completedAt/cancelledAt as Linear does.
 */
export default mutation({
  args: {
    issueId: v.id("Issue"),
    title: v.optional(v.string()),
    description: v.optional(v.string()),
    state: v.optional(v.string()),
    priority: v.optional(v.number()),
    assigneeId: v.optional(v.id("User")),
    cycleId: v.optional(v.id("Cycle")),
    projectId: v.optional(v.id("Project")),
    estimate: v.optional(v.number()),
  },
  async handler(ctx, args) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    if (!ctx.auth.tenantId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    const issue = await ctx.db.get("Issue", args.issueId);
    if (!issue || issue.orgId !== ctx.auth.tenantId)
      throw ctx.error("ISSUE_NOT_FOUND", "issue not in this org");

    const now = new Date().toISOString();
    const patch: Record<string, unknown> = { updatedAt: now };
    const activities: { kind: string; meta: Record<string, unknown> }[] = [];

    if (args.title !== undefined && args.title.trim() !== issue.title) {
      patch.title = args.title.trim();
      activities.push({ kind: "renamed", meta: { from: issue.title, to: args.title.trim() } });
    }
    if (args.description !== undefined && args.description !== issue.description) {
      patch.description = args.description;
    }
    if (args.state !== undefined && args.state !== issue.state) {
      patch.state = args.state;
      if (args.state === "in_progress" && !issue.startedAt) patch.startedAt = now;
      if (args.state === "done") patch.completedAt = now;
      if (args.state === "cancelled") patch.cancelledAt = now;
      activities.push({ kind: "state_changed", meta: { from: issue.state, to: args.state } });
    }
    if (args.priority !== undefined && args.priority !== issue.priority) {
      patch.priority = args.priority;
      activities.push({
        kind: "priority_changed",
        meta: { from: issue.priority, to: args.priority },
      });
    }
    if (args.assigneeId !== undefined && args.assigneeId !== issue.assigneeId) {
      patch.assigneeId = args.assigneeId || null;
      activities.push({
        kind: "assigned",
        meta: { from: issue.assigneeId, to: args.assigneeId || null },
      });
    }
    if (args.cycleId !== undefined && args.cycleId !== issue.cycleId) {
      patch.cycleId = args.cycleId || null;
    }
    if (args.projectId !== undefined && args.projectId !== issue.projectId) {
      patch.projectId = args.projectId || null;
    }
    if (args.estimate !== undefined && args.estimate !== issue.estimate) {
      patch.estimate = args.estimate ?? null;
      activities.push({
        kind: "estimated",
        meta: { from: issue.estimate, to: args.estimate ?? null },
      });
    }

    if (Object.keys(patch).length <= 1) return { issueId: args.issueId, changed: false };

    await ctx.db.update("Issue", args.issueId, patch);
    for (const a of activities) {
      await ctx.db.insert("IssueActivity", {
        orgId: ctx.auth.tenantId,
        issueId: args.issueId,
        actorId: ctx.auth.userId,
        kind: a.kind,
        metaJson: JSON.stringify(a.meta),
        createdAt: now,
      });
    }
    return { issueId: args.issueId, changed: true };
  },
});
