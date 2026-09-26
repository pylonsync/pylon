import { expect, test } from "bun:test";
import { FREE_PROJECT_LIMIT } from "../lib/plans";
import { SAMPLE_PROJECTS, planSampleWorkspace } from "../lib/sample-workspace";
import { isTaskPriority, isTaskStatus, normalizeTaskTitle } from "../lib/tasks";

const NOW = new Date("2026-09-26T12:00:00Z");

test("the samples leave a free workspace room for one project of its own", () => {
  const active = SAMPLE_PROJECTS.filter((p) => p.status === "active").length;
  expect(active).toBeLessThan(FREE_PROJECT_LIMIT);
});

test("every sample task passes the same validation as createTask", () => {
  for (const p of SAMPLE_PROJECTS) {
    for (const t of p.tasks) {
      expect(normalizeTaskTitle(t.title)).toBe(t.title);
      expect(isTaskStatus(t.status)).toBe(true);
      expect(isTaskPriority(t.priority)).toBe(true);
    }
  }
});

test("due dates are relative to now, and done tasks get a completion time", () => {
  const plan = planSampleWorkspace([], NOW);
  const first = plan[0].tasks.find((t) => t.title === "Fix the mobile nav overlap on Safari")!;
  expect(first.dueDate).toBe("2026-09-26");
  const done = plan[0].tasks.find((t) => t.status === "done")!;
  expect(done.completedAt).not.toBeNull();
  expect(plan[0].tasks.filter((t) => t.status !== "done").every((t) => t.completedAt === null)).toBe(true);
});

test("tasks go only to the members passed in", () => {
  const solo = planSampleWorkspace([{ userId: "u1", name: "Dana Reyes" }], NOW);
  const assigned = solo.flatMap((p) => p.tasks).filter((t) => t.assigneeId);
  expect(assigned.length).toBeGreaterThan(0);
  expect(assigned.every((t) => t.assigneeId === "u1" && t.assigneeName === "Dana Reyes")).toBe(true);

  const nobody = planSampleWorkspace([], NOW);
  expect(nobody.flatMap((p) => p.tasks).every((t) => t.assigneeId === null)).toBe(true);
});

test("a team gets the work spread across people", () => {
  const team = ["a", "b", "c", "d"].map((id) => ({ userId: id, name: id }));
  const ids = new Set(planSampleWorkspace(team, NOW).flatMap((p) => p.tasks).map((t) => t.assigneeId));
  expect(ids).toEqual(new Set(["a", "b", "c", "d", null]));
});
