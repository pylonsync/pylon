import { expect, test } from "bun:test";
import {
  compareTasks,
  daysUntil,
  isTaskPriority,
  isTaskStatus,
  normalizeDueDate,
  normalizeTaskTitle,
  progressOf,
  type TaskRow,
} from "../lib/tasks";

// Tier 1 (pure logic): the validation createTask/updateTask run on the
// server, and the ordering and progress math the dashboard renders.

test("statuses and priorities are closed lists", () => {
  expect(isTaskStatus("in_review")).toBe(true);
  expect(isTaskStatus("blocked")).toBe(false);
  expect(isTaskStatus(undefined)).toBe(false);
  expect(isTaskPriority("urgent")).toBe(true);
  expect(isTaskPriority("critical")).toBe(false);
});

test("titles are trimmed, collapsed, and bounded", () => {
  expect(normalizeTaskTitle("  Ship   the  build ")).toBe("Ship the build");
  expect(normalizeTaskTitle("   ")).toBeNull();
  expect(normalizeTaskTitle("x".repeat(200))).toHaveLength(200);
  expect(normalizeTaskTitle("x".repeat(201))).toBeNull();
});

test("due dates must be real calendar days", () => {
  expect(normalizeDueDate("2026-02-28")).toBe("2026-02-28");
  expect(normalizeDueDate("2026-02-30")).toBeNull();
  expect(normalizeDueDate("26-2-3")).toBeNull();
  expect(normalizeDueDate("")).toBeNull();
  expect(normalizeDueDate(null)).toBeNull();
});

test("progress counts done tasks", () => {
  expect(progressOf([])).toEqual({ done: 0, total: 0, percent: 0 });
  expect(progressOf([{ status: "done" }, { status: "todo" }, { status: "done" }])).toEqual({
    done: 2,
    total: 3,
    percent: 67,
  });
});

test("daysUntil is negative when overdue", () => {
  expect(daysUntil("2026-09-26", "2026-09-26")).toBe(0);
  expect(daysUntil("2026-10-01", "2026-09-26")).toBe(5);
  expect(daysUntil("2026-09-20", "2026-09-26")).toBe(-6);
});

test("board order: priority, then due date, then newest", () => {
  const row = (id: string, priority: string, dueDate: string | null, createdAt: string): TaskRow => ({
    id,
    orgId: "o",
    projectId: "p",
    title: id,
    status: "todo",
    priority,
    dueDate,
    createdAt,
  });
  const sorted = [
    row("low-soon", "low", "2026-09-27", "2026-09-01"),
    row("urgent", "urgent", null, "2026-09-01"),
    row("high-late", "high", "2026-10-09", "2026-09-01"),
    row("high-soon", "high", "2026-09-28", "2026-09-01"),
    row("high-undated-new", "high", null, "2026-09-03"),
    row("high-undated-old", "high", null, "2026-09-02"),
  ].sort(compareTasks);
  expect(sorted.map((t) => t.id)).toEqual([
    "urgent",
    "high-soon",
    "high-late",
    "high-undated-new",
    "high-undated-old",
    "low-soon",
  ]);
});
