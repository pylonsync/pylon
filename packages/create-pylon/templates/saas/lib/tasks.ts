// Task vocabulary and pure helpers. The server functions (createTask,
// updateTask) validate with these, and the dashboard renders from the same
// lists, so a status or priority can't exist in one place and not the other.
// tests/tasks.test.ts covers them without a running server.

export const TASK_STATUSES = ["todo", "in_progress", "in_review", "done"] as const;
export type TaskStatus = (typeof TASK_STATUSES)[number];

export const STATUS_LABEL: Record<TaskStatus, string> = {
  todo: "To do",
  in_progress: "In progress",
  in_review: "In review",
  done: "Done",
};

export const TASK_PRIORITIES = ["urgent", "high", "medium", "low", "none"] as const;
export type TaskPriority = (typeof TASK_PRIORITIES)[number];

export const PRIORITY_LABEL: Record<TaskPriority, string> = {
  urgent: "Urgent",
  high: "High",
  medium: "Medium",
  low: "Low",
  none: "No priority",
};

export interface TaskRow {
  id: string;
  orgId: string;
  projectId: string;
  title: string;
  status: string;
  priority?: string | null;
  assigneeId?: string | null;
  assigneeName?: string | null;
  dueDate?: string | null;
  completedAt?: string | null;
  createdAt: string;
}

export function isTaskStatus(value: unknown): value is TaskStatus {
  return typeof value === "string" && (TASK_STATUSES as readonly string[]).includes(value);
}

export function isTaskPriority(value: unknown): value is TaskPriority {
  return typeof value === "string" && (TASK_PRIORITIES as readonly string[]).includes(value);
}

/** Trimmed title, or null when it is empty or longer than 200 characters. */
export function normalizeTaskTitle(raw: string): string | null {
  const title = raw.trim().replace(/\s+/g, " ");
  if (title.length < 1 || title.length > 200) return null;
  return title;
}

/** `YYYY-MM-DD` that is a real calendar date, or null. */
export function normalizeDueDate(raw: string | null | undefined): string | null {
  if (!raw) return null;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(raw)) return null;
  const d = new Date(`${raw}T00:00:00Z`);
  if (Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== raw) return null;
  return raw;
}

/** Done / total counts and a 0–100 percentage for one project's tasks. */
export function progressOf(tasks: Pick<TaskRow, "status">[]): {
  done: number;
  total: number;
  percent: number;
} {
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === "done").length;
  return { done, total, percent: total === 0 ? 0 : Math.round((done / total) * 100) };
}

const PRIORITY_RANK: Record<string, number> = { urgent: 0, high: 1, medium: 2, low: 3, none: 4 };

/**
 * Board order inside a column: priority first, then the earliest due date,
 * then the newest task. Undated tasks sort after dated ones.
 */
export function compareTasks(a: TaskRow, b: TaskRow): number {
  const pa = PRIORITY_RANK[a.priority ?? "none"] ?? 4;
  const pb = PRIORITY_RANK[b.priority ?? "none"] ?? 4;
  if (pa !== pb) return pa - pb;
  const da = a.dueDate ?? "9999-12-31";
  const db = b.dueDate ?? "9999-12-31";
  if (da !== db) return da < db ? -1 : 1;
  return a.createdAt < b.createdAt ? 1 : -1;
}

/** Whole days from `today` to `due` (both `YYYY-MM-DD`). Negative when overdue. */
export function daysUntil(due: string, today: string): number {
  const ms = Date.parse(`${due}T00:00:00Z`) - Date.parse(`${today}T00:00:00Z`);
  return Math.round(ms / 86_400_000);
}

/** `YYYY-MM-DD` for a Date, in UTC. */
export function isoDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}
