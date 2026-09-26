import { mutation } from "@pylonsync/functions";
import { SEED_TEAM, shapeSeed } from "../lib/seed";
import { ensureDemoUser } from "../lib/demo-team";

/**
 * Fill a brand-new delivery workspace once.
 *
 * Seeds time as ENTRIES rather than totals, because logged time here is the
 * sum of the ledger; a seed that wrote totals would contradict the app's own
 * model. One project lands over its budget so that state is visible on first
 * load.
 *
 * Returns immediately if any project exists, so it is safe on every load. An
 * advisory lock stops two first loads from seeding twice. Tasks are assigned
 * across the person who signed in and four demo teammates (User rows with no
 * password, see SEED_TEAM), and each entry is logged by its task's assignee.
 * Delete this function, lib/seed.ts, and the `seedWorkspace` call in
 * app/workspace.tsx once you have real projects, then delete the demo users.
 */
export default mutation<Record<string, never>, { seeded: boolean }>({
  auth: "user",
  args: {},
  async handler(ctx) {
    if ((await ctx.db.query("Project", { $limit: 1 })).length > 0) return { seeded: false };
    await ctx.db.advisoryLock("projects_seed_workspace");
    if ((await ctx.db.query("Project", { $limit: 1 })).length > 0) return { seeded: false };

    const seed = shapeSeed();
    const me = ctx.auth.userId;

    // Position 0 is the person who signed in; SEED_TEAM follows.
    const team: Array<string | null> = [me];
    for (const person of SEED_TEAM) team.push(await ensureDemoUser(ctx.db, person));

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

    const tasks = new Map<string, { id: string; assignee: string | null }>();
    for (const task of seed.tasks) {
      const assignee = task.assignee === null ? null : (team[task.assignee] ?? me);
      const id = await ctx.db.insert("Task", {
        ...task.row,
        projectId: projectIds.get(task.project) ?? null,
        assigneeId: assignee,
      });
      tasks.set(task.key, { id: id as string, assignee });
    }

    for (const entry of seed.entries) {
      const task = tasks.get(entry.task);
      await ctx.db.insert("TimeEntry", {
        ...entry.row,
        taskId: task?.id ?? null,
        projectId: projectIds.get(entry.project) ?? null,
        userId: task?.assignee ?? me,
      });
    }

    return { seeded: true };
  },
});
