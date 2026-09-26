import { mutation, v } from "@pylonsync/functions";
import { memberName } from "../lib/members";
import {
  isTaskPriority,
  isTaskStatus,
  normalizeDueDate,
  normalizeTaskTitle,
} from "../lib/tasks";

// createTask — adds a task to a project. The workspace comes from the
// project row, never from the caller, and the caller must be a member of it.
// An assignee must be a member of the same workspace.
export default mutation<
  {
    projectId: string;
    title: string;
    status?: string;
    priority?: string;
    assigneeId?: string;
    dueDate?: string;
  },
  { id: string }
>({
  args: {
    projectId: v.string(),
    title: v.string(),
    status: v.optional(v.string()),
    priority: v.optional(v.string()),
    assigneeId: v.optional(v.string()),
    dueDate: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const title = normalizeTaskTitle(args.title);
    if (!title) throw ctx.error("INVALID_ARGS", "Task title must be 1–200 characters.");
    const status = args.status ?? "todo";
    if (!isTaskStatus(status)) throw ctx.error("INVALID_ARGS", "Unknown task status.");
    const priority = args.priority ?? "none";
    if (!isTaskPriority(priority)) throw ctx.error("INVALID_ARGS", "Unknown task priority.");
    const dueDate = args.dueDate ? normalizeDueDate(args.dueDate) : null;
    if (args.dueDate && !dueDate) throw ctx.error("INVALID_ARGS", "Due date must be YYYY-MM-DD.");

    const project = await ctx.db.unsafe.get("Project", args.projectId);
    if (!project) throw ctx.error("NOT_FOUND", "Project not found.");
    const orgId = String(project.orgId);
    await ctx.requireMember(orgId);

    let assigneeName: string | null = null;
    if (args.assigneeId) {
      assigneeName = await memberName(ctx.db, orgId, args.assigneeId);
      if (assigneeName === null) {
        throw ctx.error("INVALID_ARGS", "The assignee is not a member of this workspace.");
      }
    }

    const id = (await ctx.db.unsafe.insert("Task", {
      orgId,
      projectId: args.projectId,
      title,
      status,
      priority,
      ...(args.assigneeId ? { assigneeId: args.assigneeId, assigneeName } : {}),
      ...(dueDate ? { dueDate } : {}),
      ...(status === "done" ? { completedAt: new Date().toISOString() } : {}),
    })) as string;
    return { id };
  },
});
