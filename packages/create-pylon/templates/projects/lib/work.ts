// Projects, tasks, and time.
//
// TIME IS A LEDGER, like stock in the inventory template: a task's logged hours
// are the sum of its time entries, never a running total on the task. Two people
// logging against the same task at once is normal, and a mutable total loses one
// of them. It also means every hour on an invoice traces to who logged it and
// when — which is the question a client asks.
//
// Minutes are integers. Hours as floats produce 7.999999 on a timesheet, and
// nobody wants to explain that.
//
// Pure: no React, no `db`.

export interface Status {
  id: string;
  label: string;
  /** Done and cancelled leave the active board. */
  terminal?: boolean;
}

export const TASK_STATUSES: Status[] = [
  { id: "todo", label: "To do" },
  { id: "doing", label: "In progress" },
  { id: "review", label: "Review" },
  { id: "done", label: "Done", terminal: true },
];

export const PROJECT_STATUSES: Status[] = [
  { id: "active", label: "Active" },
  { id: "paused", label: "Paused" },
  { id: "complete", label: "Complete", terminal: true },
];

export interface Project {
  id: string;
  name: string;
  clientId?: string | null;
  status: string;
  /** Agreed budget in whole minutes, 0 for none. */
  budgetMinutes?: number | null;
  hourlyRateCents?: number | null;
  dueDate?: string | null;
  createdAt?: string | null;
}

export interface Task {
  id: string;
  projectId: string;
  title: string;
  status: string;
  assigneeId?: string | null;
  /** What we thought it would take, in minutes. */
  estimateMinutes?: number | null;
  position?: number | null;
  createdAt?: string | null;
}

export interface TimeEntry {
  id: string;
  taskId: string;
  projectId: string;
  minutes: number;
  note?: string | null;
  userId?: string | null;
  spentOn?: string | null;
}

export interface ProjectTab {
  id: string;
  label: string;
  /** Project status the tab shows; undefined shows every project. */
  status?: string;
}

/** The project list's tabs, left to right. */
export const PROJECT_TABS: ProjectTab[] = [
  { id: "active", label: "Active", status: "active" },
  { id: "paused", label: "Paused", status: "paused" },
  { id: "complete", label: "Complete", status: "complete" },
  { id: "all", label: "All" },
];

/** The tab the list opens on. The sidebar's Projects count is this tab's count. */
export const DEFAULT_PROJECT_TAB = "active";

export function inProjectTab(tabId: string, project: { status: string }): boolean {
  const tab = PROJECT_TABS.find((t) => t.id === tabId);
  return !tab?.status || project.status === tab.status;
}

/**
 * How many projects each tab lists. The sidebar badge, the page header, and
 * the tab counts all read this, so they always agree.
 */
export function projectTabCounts(projects: Array<{ status: string }>): Record<string, number> {
  return Object.fromEntries(
    PROJECT_TABS.map((tab) => [tab.id, projects.filter((p) => inProjectTab(tab.id, p)).length]),
  );
}

export function taskStatusById(id: string): Status | undefined {
  return TASK_STATUSES.find((s) => s.id === id);
}

export function isValidTaskStatus(id: string): boolean {
  return TASK_STATUSES.some((s) => s.id === id);
}

export function isValidProjectStatus(id: string): boolean {
  return PROJECT_STATUSES.some((s) => s.id === id);
}

export function isOpen(task: { status: string }): boolean {
  return !taskStatusById(task.status)?.terminal;
}

export interface Column {
  status: Status;
  tasks: Task[];
}

/** Group a project's tasks into board columns, in their manual order. */
export function groupByStatus(tasks: Task[]): Column[] {
  return TASK_STATUSES.map((status) => ({
    status,
    tasks: tasks
      .filter((task) => task.status === status.id)
      .sort(
        (a, b) =>
          (Number(a.position) || 0) - (Number(b.position) || 0) ||
          (a.createdAt ?? "").localeCompare(b.createdAt ?? ""),
      ),
  }));
}

/** Minutes logged against one task. */
export function minutesForTask(taskId: string, entries: TimeEntry[]): number {
  let total = 0;
  for (const entry of entries) {
    if (entry.taskId !== taskId) continue;
    const minutes = Number(entry.minutes);
    if (Number.isFinite(minutes)) total += Math.trunc(minutes);
  }
  return total;
}

/**
 * Minutes per task in one pass — a board calling minutesForTask per card would
 * be O(tasks × entries).
 */
export function minutesByTask(entries: TimeEntry[]): Map<string, number> {
  const totals = new Map<string, number>();
  for (const entry of entries) {
    const minutes = Number(entry.minutes);
    if (!Number.isFinite(minutes)) continue;
    totals.set(entry.taskId, (totals.get(entry.taskId) ?? 0) + Math.trunc(minutes));
  }
  return totals;
}

export function minutesForProject(projectId: string, entries: TimeEntry[]): number {
  let total = 0;
  for (const entry of entries) {
    if (entry.projectId !== projectId) continue;
    const minutes = Number(entry.minutes);
    if (Number.isFinite(minutes)) total += Math.trunc(minutes);
  }
  return total;
}

export interface Progress {
  total: number;
  done: number;
  /** 0–1, or null when there's nothing to measure. */
  ratio: number | null;
}

/**
 * Progress by task COUNT, not by logged time.
 *
 * Time spent measures effort, not completion — a project can be 90% through its
 * budget and 20% done, and conflating them is how a status report lies.
 * `budgetState` reports the other half separately.
 */
export function progress(tasks: Task[]): Progress {
  const total = tasks.length;
  const done = tasks.filter((task) => !isOpen(task)).length;
  return { total, done, ratio: total === 0 ? null : done / total };
}

export type BudgetState = "none" | "ok" | "near" | "over";

/** Where a project sits against its agreed budget. */
export function budgetState(
  budgetMinutes: number | null | undefined,
  loggedMinutes: number,
): BudgetState {
  const budget = Number(budgetMinutes);
  if (!Number.isFinite(budget) || budget <= 0) return "none";
  if (loggedMinutes > budget) return "over";
  // 85% is where you can still do something about it.
  if (loggedMinutes >= budget * 0.85) return "near";
  return "ok";
}

/** What the logged time is worth at the project's rate. */
export function billableCents(
  minutes: number,
  hourlyRateCents: number | null | undefined,
): number {
  const rate = Number(hourlyRateCents) || 0;
  return Math.round((minutes * rate) / 60);
}

/** "7h 30m" / "45m" / "0m" — the format a timesheet is read in. */
export function duration(minutes: number | null | undefined): string {
  const value = Math.trunc(Number(minutes) || 0);
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const hours = Math.floor(abs / 60);
  const rest = abs % 60;
  if (hours === 0) return `${sign}${rest}m`;
  if (rest === 0) return `${sign}${hours}h`;
  return `${sign}${hours}h ${rest}m`;
}

export function money(cents: number | null | undefined): string {
  const value = Number(cents) || 0;
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const dollars = Math.floor(abs / 100).toLocaleString("en-US");
  const rest = String(abs % 100).padStart(2, "0");
  return `${sign}$${dollars}.${rest}`;
}

/**
 * Parse typed time into minutes. Accepts "90", "1.5h", "1h30", "1h 30m", "45m".
 *
 * People write time however they think of it, and rejecting "1h30" because it
 * isn't "90" is the kind of friction that stops time being logged at all —
 * which costs far more than a lenient parser.
 */
export function parseDuration(input: string): number | null {
  const text = input.trim().toLowerCase().replace(/\s+/g, "");
  if (!text) return null;

  // "1h30" / "1h30m" / "2h" / "45m"
  const hm = text.match(/^(\d+(?:\.\d+)?)h(?:(\d+)m?)?$/);
  if (hm) {
    const hours = Number(hm[1]);
    const mins = hm[2] ? Number(hm[2]) : 0;
    if (!Number.isFinite(hours) || !Number.isFinite(mins) || mins >= 60) return null;
    return Math.round(hours * 60 + mins);
  }

  const m = text.match(/^(\d+)m$/);
  if (m) return Number(m[1]);

  // A bare number is minutes — the unit a timesheet field is usually in.
  if (/^\d+(\.\d+)?$/.test(text)) {
    const value = Number(text);
    return Number.isFinite(value) ? Math.round(value) : null;
  }

  return null;
}

export interface Portfolio {
  /** Projects with status "active". */
  activeCount: number;
  /** Minutes logged against active projects. */
  loggedMinutes: number;
  /** What that time is worth at each project's rate. */
  billableCents: number;
  /** Active projects past their budget. */
  overBudgetCount: number;
}

/** The numbers above the project list, over active projects only. */
export function portfolio(projects: Project[], entries: TimeEntry[]): Portfolio {
  const result: Portfolio = {
    activeCount: 0,
    loggedMinutes: 0,
    billableCents: 0,
    overBudgetCount: 0,
  };
  for (const project of projects) {
    if (project.status !== "active") continue;
    const minutes = minutesForProject(project.id, entries);
    result.activeCount += 1;
    result.loggedMinutes += minutes;
    result.billableCents += billableCents(minutes, project.hourlyRateCents);
    if (budgetState(project.budgetMinutes, minutes) === "over") result.overBudgetCount += 1;
  }
  return result;
}
