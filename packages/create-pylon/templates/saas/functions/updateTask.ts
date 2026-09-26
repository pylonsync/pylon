import { mutation, v } from "@pylonsync/functions";
import { memberName } from "../lib/members";
import {
  isTaskPriority,
  isTaskStatus,
  normalizeDueDate,
  normalizeTaskTitle,
} from "../lib/tasks";

// updateTask — changes the fields a person edits on the board. Only the
// fields passed are written. An empty string clears `assigneeId` or
// `dueDate`. The caller must be a member of the task's own workspace, and the
// task can never move to another project or workspace.
export default mutation<
  {
    taskId: string;
    title?: string;
    status?: string;
    priority?: string;
    assigneeId?: string;
    dueDate?: string;
  },
  { id: string }
>({
  args: {
    taskId: v.string(),
    title: v.optional(v.string()),
    status: v.optional(v.string()),
    priority: v.optional(v.string()),
    assigneeId: v.optional(v.string()),
    dueDate: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const task = await ctx.db.unsafe.get("Task", args.taskId);
    if (!task) throw ctx.error("NOT_FOUND", "Task not found.");
    const orgId = String(task.orgId);
    await ctx.requireMember(orgId);

    const patch: Record<string, unknown> = {};
    if (args.title !== undefined) {
      const title = normalizeTaskTitle(args.title);
      if (!title) throw ctx.error("INVALID_ARGS", "Task title must be 1–200 characters.");
      patch.title = title;
    }
    if (args.status !== undefined) {
      if (!isTaskStatus(args.status)) throw ctx.error("INVALID_ARGS", "Unknown task status.");
      patch.status = args.status;
      if (args.status === "done" && task.status !== "done") {
        patch.completedAt = new Date().toISOString();
      } else if (args.status !== "done") {
        patch.completedAt = null;
      }
    }
    if (args.priority !== undefined) {
      if (!isTaskPriority(args.priority)) throw ctx.error("INVALID_ARGS", "Unknown task priority.");
      patch.priority = args.priority;
    }
    if (args.assigneeId !== undefined) {
      if (args.assigneeId === (task.assigneeId ?? "")) {
        // Unchanged. The dialog sends every field, and a person who has since
        // left the workspace stays assigned until someone picks a new owner.
      } else if (args.assigneeId === "") {
        patch.assigneeId = null;
        patch.assigneeName = null;
      } else {
        const name = await memberName(ctx.db, orgId, args.assigneeId);
        if (name === null) {
          throw ctx.error("INVALID_ARGS", "The assignee is not a member of this workspace.");
        }
        patch.assigneeId = args.assigneeId;
        patch.assigneeName = name;
      }
    }
    if (args.dueDate !== undefined) {
      if (args.dueDate === "") {
        patch.dueDate = null;
      } else {
        const due = normalizeDueDate(args.dueDate);
        if (!due) throw ctx.error("INVALID_ARGS", "Due date must be YYYY-MM-DD.");
        patch.dueDate = due;
      }
    }
    if (Object.keys(patch).length > 0) {
      await ctx.db.unsafe.update("Task", args.taskId, patch);
    }
    return { id: args.taskId };
  },
});
