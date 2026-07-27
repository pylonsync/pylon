import { mutation } from "@pylonsync/functions";
import { shapeSeed } from "../lib/seed";

/**
 * Fill a brand-new delivery workspace once.
 *
 * Seeds time as ENTRIES rather than totals, because logged time here is the sum
 * of the ledger — a seed that wrote totals would contradict the app\'s own model.
 * One project lands over its budget so that state is visible on first load.
 *
 * Returns immediately if any project exists, so it is safe on every load.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    const existing = await ctx.db.query("Project", { $limit: 1 });
    if (existing.length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    const clientIds = new Map<string, string>();
    for (const client of seed.clients) {
      const id = await ctx.db.insert("Client", client.row);
      clientIds.set(client.key, id as string);
    }

    const projectIds = new Map<string, string>();
    for (const project of seed.projects) {
      const id = await ctx.db.insert("Project", {
        ...project.row,
        clientId: clientIds.get(project.client) ?? null,
      });
      projectIds.set(project.key, id as string);
    }

    const taskIds = new Map<string, string>();
    for (const task of seed.tasks) {
      const id = await ctx.db.insert("Task", {
        ...task.row,
        projectId: projectIds.get(task.project) ?? null,
        assigneeId: me,
      });
      taskIds.set(task.key, id as string);
    }

    for (const entry of seed.entries) {
      await ctx.db.insert("TimeEntry", {
        ...entry.row,
        taskId: taskIds.get(entry.task) ?? null,
        projectId: projectIds.get(entry.project) ?? null,
        userId: me,
      });
    }

    return { seeded: true };
  },
});
