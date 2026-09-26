import { mutation, v } from "@pylonsync/functions";
import { workspaceMembers } from "../lib/members";
import { planSampleWorkspace } from "../lib/sample-workspace";

// seedWorkspace — fills an empty workspace with the sample projects and
// tasks in lib/sample-workspace.ts. Owners and admins only. It does nothing
// when the workspace already has a project, so calling it twice is safe.
// Tasks are assigned to the workspace's current members.
export default mutation<{ orgId: string }, { seeded: boolean; projects: number; tasks: number }>({
  args: { orgId: v.string() },
  async handler(ctx, args) {
    await ctx.requireMember(args.orgId, { role: ["owner", "admin"] });

    const existing = await ctx.db.unsafe.query("Project", { orgId: args.orgId });
    if (existing.length > 0) return { seeded: false, projects: 0, tasks: 0 };

    const members = await workspaceMembers(ctx.db, args.orgId);
    // The caller first, so the tasks marked for member 0 land on them.
    members.sort((a, b) => (a.userId === ctx.auth.userId ? -1 : b.userId === ctx.auth.userId ? 1 : 0));
    const plan = planSampleWorkspace(members, new Date());

    let taskCount = 0;
    for (const p of plan) {
      const projectId = (await ctx.db.unsafe.insert("Project", {
        orgId: args.orgId,
        name: p.name,
        description: p.description,
        status: p.status,
      })) as string;
      for (const t of p.tasks) {
        await ctx.db.unsafe.insert("Task", {
          orgId: args.orgId,
          projectId,
          title: t.title,
          status: t.status,
          priority: t.priority,
          dueDate: t.dueDate,
          ...(t.completedAt ? { completedAt: t.completedAt } : {}),
          ...(t.assigneeId ? { assigneeId: t.assigneeId, assigneeName: t.assigneeName } : {}),
        });
        taskCount++;
      }
    }
    return { seeded: true, projects: plan.length, tasks: taskCount };
  },
});
