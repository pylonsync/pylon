import { mutation } from "@pylonsync/functions";

/**
 * Populate the caller's active org with a demo team and a spread of issues
 * across every workflow state, so a fresh workspace opens to a populated
 * board instead of an empty one. Idempotent: returns early if the org already
 * has a team, so it's safe to call whenever an empty workspace is opened.
 */
export default mutation({
  auth: "guest",
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) throw ctx.error("UNAUTHENTICATED", "log in first");
    const orgId = ctx.auth.tenantId;
    if (!orgId) throw ctx.error("NO_ACTIVE_ORG", "select an org");

    // Gate on issues, not teams: createOrganization already creates a default
    // "Engineering" team, so seed issues into that existing team rather than
    // adding a second one (which would also collide on the unique team key).
    const existingIssues = await ctx.db.query("Issue", { orgId });
    if (existingIssues.length > 0) return { seeded: false, reason: "already has data" };

    const teams = await ctx.db.query("Team", { orgId });
    if (teams.length === 0) return { seeded: false, reason: "no team to seed into" };
    const teamId = teams[0].id;
    if (typeof teamId !== "string")
      throw ctx.error("INTERNAL", "team record is missing its id");

    const userId = ctx.auth.userId;
    const now = Date.now();
    const iso = (offsetMin: number) => new Date(now + offsetMin * 60_000).toISOString();

    // title, state, priority(0 none → 4 urgent), createdOffsetMinutes
    const issues: Array<{ title: string; state: string; priority: number; created: number; estimate?: number }> = [
      { title: "Set up CI pipeline for the monorepo", state: "done", priority: 2, created: -60 * 24 * 10, estimate: 3 },
      { title: "Design the sync engine reconnect backoff", state: "done", priority: 3, created: -60 * 24 * 8, estimate: 5 },
      { title: "Add optimistic updates to the issue board", state: "in_review", priority: 2, created: -60 * 24 * 4, estimate: 5 },
      { title: "Live presence avatars in the top bar", state: "in_progress", priority: 1, created: -60 * 24 * 3, estimate: 3 },
      { title: "Keyboard-first command palette (⌘K)", state: "in_progress", priority: 2, created: -60 * 24 * 2, estimate: 2 },
      { title: "Triage: flaky drag-and-drop on Safari", state: "triage", priority: 3, created: -60 * 12 },
      { title: "Cycle burndown chart", state: "todo", priority: 1, created: -60 * 8, estimate: 8 },
      { title: "Bulk-edit issues from the list view", state: "todo", priority: 2, created: -60 * 5 },
      { title: "Markdown shortcuts in the description editor", state: "backlog", priority: 0, created: -60 * 3 },
      { title: "SAML SSO for enterprise workspaces", state: "backlog", priority: 1, created: -60 * 2 },
    ];

    let seq = 0;
    for (const it of issues) {
      seq += 1;
      const createdAt = iso(it.created);
      const issueId = await ctx.db.insert("Issue", {
        orgId,
        teamId,
        number: seq,
        title: it.title,
        description: null,
        state: it.state,
        priority: it.priority,
        assigneeId: it.state === "backlog" ? null : userId,
        creatorId: userId,
        cycleId: null,
        projectId: null,
        estimate: it.estimate ?? null,
        createdAt,
        updatedAt: createdAt,
        startedAt: ["in_progress", "in_review", "done"].includes(it.state) ? createdAt : null,
        completedAt: it.state === "done" ? createdAt : null,
        cancelledAt: null,
      });
      await ctx.db.insert("IssueActivity", {
        orgId,
        issueId,
        actorId: userId,
        kind: "created",
        metaJson: null,
        createdAt,
      });
    }
    await ctx.db.update("Team", teamId, { issueSequence: seq });

    return { seeded: true, team: "ENG", issues: issues.length };
  },
});
