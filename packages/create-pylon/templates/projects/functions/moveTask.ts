import { mutation, v } from "@pylonsync/functions";
import { isValidTaskStatus } from "../lib/work";

/**
 * Move a task to another column.
 *
 * Validated server-side: the board only sends known statuses, but the endpoint
 * is reachable directly and an unknown status would strand the task in a column
 * that never renders.
 *
 * The new position is the end of the target column, computed here rather than
 * sent by the client — two people dragging into the same column simultaneously
 * would otherwise both claim the same index.
 */
export default mutation<{ taskId: string; status: string }, { ok: true }>({
  auth: "user",
  args: { taskId: v.id("Task"), status: v.string() },
  async handler(ctx, args) {
    if (!isValidTaskStatus(args.status)) {
      throw ctx.error("INVALID_ARGS", `Unknown status "${args.status}".`);
    }

    const task = await ctx.db.get("Task", args.taskId);
    if (!task) throw ctx.error("NOT_FOUND", "Task not found.");
    if (task.status === args.status) return { ok: true } as const;

    const siblings = (await ctx.db.query("Task", {
      projectId: task.projectId,
    })) as Array<{ status?: string; position?: number }>;
    const end = siblings.filter((row) => row.status === args.status).length;

    await ctx.db.update("Task", args.taskId, {
      status: args.status,
      position: end,
      updatedAt: new Date().toISOString(),
    });

    return { ok: true } as const;
  },
});
