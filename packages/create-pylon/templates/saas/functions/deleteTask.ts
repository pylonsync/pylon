import { mutation, v } from "@pylonsync/functions";

// deleteTask — removes one task. The caller must be a member of the task's
// own workspace.
export default mutation<{ taskId: string }, { ok: true }>({
  args: { taskId: v.string() },
  async handler(ctx, args) {
    const task = await ctx.db.unsafe.get("Task", args.taskId);
    if (!task) throw ctx.error("NOT_FOUND", "Task not found.");
    await ctx.requireMember(String(task.orgId));
    await ctx.db.unsafe.delete("Task", args.taskId);
    return { ok: true };
  },
});
