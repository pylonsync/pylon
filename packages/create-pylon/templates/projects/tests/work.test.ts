import { describe, expect, test } from "bun:test";
import {
  TASK_STATUSES,
  billableCents,
  budgetState,
  duration,
  groupByStatus,
  isOpen,
  isValidProjectStatus,
  isValidTaskStatus,
  minutesByTask,
  minutesForProject,
  minutesForTask,
  money,
  parseDuration,
  progress,
  type Task,
  type TimeEntry,
} from "../lib/work";

const task = (over: Partial<Task> = {}): Task => ({
  id: "t1",
  projectId: "p1",
  title: "Do the thing",
  status: "todo",
  ...over,
});

const entry = (over: Partial<TimeEntry> = {}): TimeEntry => ({
  id: "e1",
  taskId: "t1",
  projectId: "p1",
  minutes: 60,
  ...over,
});

describe("statuses", () => {
  test("only done is terminal on the board", () => {
    expect(isOpen(task({ status: "todo" }))).toBe(true);
    expect(isOpen(task({ status: "doing" }))).toBe(true);
    expect(isOpen(task({ status: "review" }))).toBe(true);
    expect(isOpen(task({ status: "done" }))).toBe(false);
  });

  test("unknown values are rejected, not coerced", () => {
    expect(isValidTaskStatus("doing")).toBe(true);
    expect(isValidTaskStatus("Doing")).toBe(false);
    expect(isValidTaskStatus("blocked")).toBe(false);
    expect(isValidProjectStatus("active")).toBe(true);
    expect(isValidProjectStatus("archived")).toBe(false);
  });

  test("the board has all four columns", () => {
    expect(TASK_STATUSES.map((s) => s.id)).toEqual(["todo", "doing", "review", "done"]);
  });
});

describe("groupByStatus", () => {
  test("puts each task in its column, in manual order", () => {
    const columns = groupByStatus([
      task({ id: "b", status: "todo", position: 1 }),
      task({ id: "a", status: "todo", position: 0 }),
      task({ id: "c", status: "done" }),
    ]);
    expect(columns[0].tasks.map((t) => t.id)).toEqual(["a", "b"]);
    expect(columns[3].tasks.map((t) => t.id)).toEqual(["c"]);
  });

  test("an unknown status is dropped rather than filed under To do", () => {
    // Silently reclassifying hides a typo behind a plausible board.
    const columns = groupByStatus([task({ status: "blocked" })]);
    expect(columns.every((c) => c.tasks.length === 0)).toBe(true);
  });

  test("always returns every column, even empty", () => {
    expect(groupByStatus([])).toHaveLength(4);
  });
});

describe("time as a ledger", () => {
  test("a task's time is the sum of its entries", () => {
    expect(
      minutesForTask("t1", [
        entry({ id: "a", minutes: 180 }),
        entry({ id: "b", minutes: 270 }),
      ]),
    ).toBe(450);
  });

  test("two people logging at once both count", () => {
    // The bug this design prevents: a mutable total loses one of them.
    expect(
      minutesForTask("t1", [
        entry({ id: "a", minutes: 60, userId: "u1" }),
        entry({ id: "b", minutes: 60, userId: "u2" }),
      ]),
    ).toBe(120);
  });

  test("a negative entry corrects without hiding the original", () => {
    const entries = [entry({ id: "a", minutes: 120 }), entry({ id: "b", minutes: -30 })];
    expect(minutesForTask("t1", entries)).toBe(90);
    expect(entries).toHaveLength(2);
  });

  test("ignores other tasks", () => {
    expect(minutesForTask("t1", [entry({ taskId: "t2", minutes: 999 })])).toBe(0);
  });

  test("minutesByTask matches the per-task version", () => {
    const entries = [
      entry({ id: "a", taskId: "t1", minutes: 60 }),
      entry({ id: "b", taskId: "t2", minutes: 30 }),
    ];
    const map = minutesByTask(entries);
    expect(map.get("t1")).toBe(minutesForTask("t1", entries));
    expect(map.get("t2")).toBe(minutesForTask("t2", entries));
  });

  test("project time sums across its tasks", () => {
    expect(
      minutesForProject("p1", [
        entry({ id: "a", taskId: "t1", minutes: 60 }),
        entry({ id: "b", taskId: "t2", minutes: 90 }),
        entry({ id: "c", projectId: "p2", minutes: 999 }),
      ]),
    ).toBe(150);
  });

  test("a corrupt entry doesn't poison the total", () => {
    expect(
      minutesForTask("t1", [
        entry({ minutes: 60 }),
        entry({ id: "x", minutes: NaN as unknown as number }),
      ]),
    ).toBe(60);
  });
});

describe("progress", () => {
  test("measures completion by task COUNT, not by time", () => {
    // A project can be 90% through its budget and 20% done; conflating them is
    // how a status report lies.
    const p = progress([
      task({ id: "a", status: "done" }),
      task({ id: "b", status: "doing" }),
      task({ id: "c", status: "todo" }),
      task({ id: "d", status: "review" }),
    ]);
    expect(p.done).toBe(1);
    expect(p.total).toBe(4);
    expect(p.ratio).toBe(0.25);
  });

  test("no tasks is null, not zero percent", () => {
    // "Nothing planned" and "nothing done" are different answers.
    expect(progress([]).ratio).toBeNull();
  });
});

describe("budgetState", () => {
  test("over once logged passes the budget", () => {
    expect(budgetState(2400, 2760)).toBe("over");
  });

  test("near at 85%", () => {
    expect(budgetState(1000, 850)).toBe("near");
    expect(budgetState(1000, 849)).toBe("ok");
  });

  test("exactly on budget is not yet over", () => {
    expect(budgetState(1000, 1000)).toBe("near");
  });

  test("no budget means nothing to be over", () => {
    expect(budgetState(0, 5000)).toBe("none");
    expect(budgetState(null, 5000)).toBe("none");
  });
});

describe("billableCents", () => {
  test("prices logged minutes at the hourly rate", () => {
    // 90 minutes at $165/h
    expect(billableCents(90, 16_500)).toBe(24_750);
  });

  test("no rate is no value", () => {
    expect(billableCents(600, 0)).toBe(0);
    expect(billableCents(600, null)).toBe(0);
  });
});

describe("duration", () => {
  test("reads the way a timesheet does", () => {
    expect(duration(450)).toBe("7h 30m");
    expect(duration(45)).toBe("45m");
    expect(duration(120)).toBe("2h");
    expect(duration(0)).toBe("0m");
  });

  test("handles nothing and negatives", () => {
    expect(duration(null)).toBe("0m");
    expect(duration(-90)).toBe("-1h 30m");
  });
});

describe("parseDuration", () => {
  test("accepts however people write time", () => {
    // Rejecting "1h30" because it isn't "90" is friction that stops time being
    // logged at all.
    expect(parseDuration("90")).toBe(90);
    expect(parseDuration("1.5h")).toBe(90);
    expect(parseDuration("1h30")).toBe(90);
    expect(parseDuration("1h30m")).toBe(90);
    expect(parseDuration("1h 30m")).toBe(90);
    expect(parseDuration("45m")).toBe(45);
    expect(parseDuration("2h")).toBe(120);
  });

  test("is case- and space-insensitive", () => {
    expect(parseDuration(" 1H30M ")).toBe(90);
  });

  test("rejects nonsense rather than logging zero", () => {
    expect(parseDuration("")).toBeNull();
    expect(parseDuration("abc")).toBeNull();
    expect(parseDuration("1h75")).toBeNull();
    expect(parseDuration("h")).toBeNull();
  });
});

describe("money", () => {
  test("is exact", () => {
    expect(money(24_750)).toBe("$247.50");
    expect(money(0)).toBe("$0.00");
    expect(money(null)).toBe("$0.00");
  });
});
