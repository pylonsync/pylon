import { mutation, v } from "@pylonsync/functions";

/**
 * Create an issue in a team. Server bumps the team's issueSequence so
 * each issue gets a monotonic per-team number — the `ENG-42` identifier
 * users actually quote. A race between two concurrent creates could
 * theoretically collide; the `by_team_number` unique index catches that
 * and the caller can retry.
 */
export default mutation({
  args: {
    teamId: v.id("Team"),
    title: v.string(),
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

    const title = args.title.trim();
    if (title.length === 0) throw ctx.error("INVALID_TITLE", "title required");
    if (title.length > 500)
      throw ctx.error("TITLE_TOO_LONG", "title capped at 500 chars");

    const team = await ctx.db.get("Team", args.teamId);
    if (!team || team.orgId !== ctx.auth.tenantId)
      throw ctx.error("TEAM_NOT_FOUND", "team not in this org");

    const validStates = [
      "backlog", "todo", "in_progress", "in_review", "done", "cancelled", "triage",
    ];
    const state = args.state || "todo";
    if (!validStates.includes(state))
      throw ctx.error("INVALID_STATE", `state ∈ ${validStates.join(", ")}`);

    const priority = args.priority ?? 0;
    if (priority < 0 || priority > 4)
      throw ctx.error("INVALID_PRIORITY", "priority ∈ 0..4");

    const now = new Date().toISOString();
    const nextNumber = (team.issueSequence ?? 0) + 1;
    await ctx.db.update("Team", team.id, { issueSequence: nextNumber });

    const issueId = await ctx.db.insert("Issue", {
      orgId: ctx.auth.tenantId,
      teamId: args.teamId,
      number: nextNumber,
      title,
      description: args.description?.trim() || null,
      state,
      priority,
      assigneeId: args.assigneeId || null,
      creatorId: ctx.auth.userId,
      cycleId: args.cycleId || null,
      projectId: args.projectId || null,
      estimate: args.estimate ?? null,
      createdAt: now,
      updatedAt: now,
      startedAt: state === "in_progress" ? now : null,
      completedAt: state === "done" ? now : null,
      cancelledAt: state === "cancelled" ? now : null,
    });

    await ctx.db.insert("IssueActivity", {
      orgId: ctx.auth.tenantId,
      issueId,
      actorId: ctx.auth.userId,
      kind: "created",
      metaJson: null,
      createdAt: now,
    });

    return { issueId, number: nextNumber, identifier: `${team.key}-${nextNumber}` };
  },
});
