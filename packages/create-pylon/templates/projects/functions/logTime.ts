import { mutation, v } from "@pylonsync/functions";

/**
 * Log time against a task.
 *
 * A mutation rather than a bare insert so `projectId` is taken from the TASK
 * rather than trusted from the client — a mislabelled entry would land on
 * another client\'s invoice, which is the worst failure this app has.
 *
 * Entries are append-only (see the policy). A correction is a NEGATIVE entry,
 * which stays visible in the history rather than quietly changing what someone
 * was billed.
 */
export default mutation<
  { taskId: string; minutes: number; note?: string },
  { ok: true; id: string }
>({
  auth: "user",
  args: {
    taskId: v.id("Task"),
    minutes: v.number(),
    note: v.optional(v.string()),
  },
  async handler(ctx, args) {
    const minutes = Math.trunc(Number(args.minutes) || 0);
    if (minutes === 0) {
      throw ctx.error("INVALID_ARGS", "Log a non-zero number of minutes.");
    }
    // A day has 1440 minutes; anything past that is a typo, and a typo on a
    // timesheet becomes a typo on an invoice.
    if (Math.abs(minutes) > 1440) {
      throw ctx.error("INVALID_ARGS", "That's more than a day — check the value.");
    }

    const task = await ctx.db.get("Task", args.taskId);
    if (!task) throw ctx.error("NOT_FOUND", "Task not found.");

    const id = await ctx.db.insert("TimeEntry", {
      taskId: args.taskId,
      projectId: task.projectId,
      minutes,
      note: args.note?.trim() || null,
      userId: ctx.auth.userId,
      spentOn: new Date().toISOString(),
    });

    return { ok: true, id: id as string } as const;
  },
});
