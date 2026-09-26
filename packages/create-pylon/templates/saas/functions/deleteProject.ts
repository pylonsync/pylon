import { mutation, v } from "@pylonsync/functions";

// deleteProject — removes a project and every task in it. The caller must be
// a member of the project's own workspace.
export default mutation<{ projectId: string }, { ok: true; tasks: number }>({
  args: { projectId: v.string() },
  async handler(ctx, args) {
    const project = await ctx.db.unsafe.get("Project", args.projectId);
    if (!project) throw ctx.error("NOT_FOUND", "Project not found.");
    await ctx.requireMember(String(project.orgId));

    const tasks = await ctx.db.unsafe.query("Task", { projectId: args.projectId });
    for (const t of tasks) await ctx.db.unsafe.delete("Task", String(t.id));
    await ctx.db.unsafe.delete("Project", args.projectId);
    return { ok: true, tasks: tasks.length };
  },
});
