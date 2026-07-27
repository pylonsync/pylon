// Demo data for a brand-new delivery workspace.
//
// An empty project list shows nothing about boards, budgets, or time. The first
// sign-in seeds two live projects — one comfortably inside its budget, one over
// it — so the budget states are visible rather than theoretical.

export interface SeedClient {
  key: string;
  name: string;
  email: string;
}

export interface SeedProject {
  key: string;
  client: string;
  name: string;
  status: string;
  /** Agreed budget in hours. */
  budgetHours: number;
  hourlyRate: number;
  dueInDays: number;
}

export interface SeedTask {
  key: string;
  project: string;
  title: string;
  status: string;
  estimateHours: number;
  /** Hours logged against it; each becomes one or more entries. */
  loggedHours: number[];
}

export const SEED_CLIENTS: SeedClient[] = [
  { key: "northwind", name: "Northwind Logistics", email: "dana@northwind.co" },
  { key: "hallmark", name: "Hallmark Dental", email: "priya@hallmarkdental.com" },
];

export const SEED_PROJECTS: SeedProject[] = [
  {
    key: "dispatch",
    client: "northwind",
    name: "Dispatch portal",
    status: "active",
    budgetHours: 80,
    hourlyRate: 165,
    dueInDays: 24,
  },
  // Over budget: 40h agreed, ~46h logged.
  {
    key: "intake",
    client: "hallmark",
    name: "Patient intake rebuild",
    status: "active",
    budgetHours: 40,
    hourlyRate: 150,
    dueInDays: 9,
  },
];

export const SEED_TASKS: SeedTask[] = [
  { key: "d-auth", project: "dispatch", title: "SSO with their identity provider", status: "done", estimateHours: 8, loggedHours: [3, 4.5] },
  { key: "d-model", project: "dispatch", title: "Route + stop data model", status: "done", estimateHours: 6, loggedHours: [5] },
  { key: "d-board", project: "dispatch", title: "Dispatcher board UI", status: "doing", estimateHours: 20, loggedHours: [6, 4] },
  { key: "d-mobile", project: "dispatch", title: "Driver mobile view", status: "review", estimateHours: 12, loggedHours: [9] },
  { key: "d-report", project: "dispatch", title: "Weekly utilisation report", status: "todo", estimateHours: 6, loggedHours: [] },
  { key: "d-import", project: "dispatch", title: "Import legacy stop history", status: "todo", estimateHours: 10, loggedHours: [] },

  { key: "i-forms", project: "intake", title: "Form builder", status: "done", estimateHours: 16, loggedHours: [8, 7, 4] },
  { key: "i-sig", project: "intake", title: "Consent signatures", status: "done", estimateHours: 8, loggedHours: [6, 5] },
  { key: "i-sync", project: "intake", title: "Sync to practice software", status: "doing", estimateHours: 12, loggedHours: [9, 7] },
  { key: "i-train", project: "intake", title: "Staff training session", status: "todo", estimateHours: 4, loggedHours: [] },
];

export interface ShapedSeed {
  clients: Array<{ key: string; row: Record<string, unknown> }>;
  projects: Array<{ key: string; client: string; row: Record<string, unknown> }>;
  tasks: Array<{ key: string; project: string; row: Record<string, unknown> }>;
  entries: Array<{ task: string; project: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const at = (days: number) => new Date(now + days * 86_400_000).toISOString();

  const tasks: ShapedSeed["tasks"] = [];
  const entries: ShapedSeed["entries"] = [];
  const positions = new Map<string, number>();

  SEED_TASKS.forEach((task, index) => {
    const columnKey = `${task.project}:${task.status}`;
    const position = positions.get(columnKey) ?? 0;
    positions.set(columnKey, position + 1);

    tasks.push({
      key: task.key,
      project: task.project,
      row: {
        title: task.title,
        status: task.status,
        estimateMinutes: Math.round(task.estimateHours * 60),
        position,
        createdAt: at(-30 + index),
        updatedAt: at(-2),
      },
    });

    task.loggedHours.forEach((hours, entryIndex) => {
      entries.push({
        task: task.key,
        project: task.project,
        row: {
          minutes: Math.round(hours * 60),
          note: null,
          spentOn: at(-14 + entryIndex * 2),
        },
      });
    });
  });

  return {
    clients: SEED_CLIENTS.map((c) => ({
      key: c.key,
      row: { name: c.name, email: c.email, createdAt: at(-90) },
    })),
    projects: SEED_PROJECTS.map((p) => ({
      key: p.key,
      client: p.client,
      row: {
        name: p.name,
        status: p.status,
        budgetMinutes: Math.round(p.budgetHours * 60),
        hourlyRateCents: Math.round(p.hourlyRate * 100),
        dueDate: at(p.dueInDays),
        createdAt: at(-45),
      },
    })),
    tasks,
    entries,
  };
}
