import { describe, expect, test } from "bun:test";
import {
  PRIORITIES,
  STATUSES,
  applyFilter,
  counts,
  isOpen,
  isValidPriority,
  isValidStatus,
  minutesToBreach,
  queueOrder,
  slaState,
  ticketNumber,
  type Ticket,
} from "../lib/tickets";

const NOW = Date.parse("2026-07-27T12:00:00Z");
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString();

const ticket = (over: Partial<Ticket> = {}): Ticket => ({
  id: "t1",
  subject: "Something broke",
  status: "open",
  priority: "normal",
  createdAt: hoursAgo(1),
  ...over,
});

describe("statuses and priorities", () => {
  test("solved and closed are terminal", () => {
    expect(isOpen(ticket({ status: "open" }))).toBe(true);
    expect(isOpen(ticket({ status: "pending" }))).toBe(true);
    expect(isOpen(ticket({ status: "solved" }))).toBe(false);
    expect(isOpen(ticket({ status: "closed" }))).toBe(false);
  });

  test("unknown values are rejected, not coerced", () => {
    expect(isValidStatus("open")).toBe(true);
    expect(isValidStatus("Open")).toBe(false);
    expect(isValidStatus("archived")).toBe(false);
    expect(isValidPriority("urgent")).toBe(true);
    expect(isValidPriority("critical")).toBe(false);
  });

  test("priorities are ranked urgent-first with tightening windows", () => {
    const ranks = PRIORITIES.map((p) => p.rank);
    expect(ranks).toEqual([...ranks].sort((a, b) => a - b));
    expect(PRIORITIES[0].id).toBe("urgent");
    expect(PRIORITIES[0].firstResponseHours).toBeLessThan(
      PRIORITIES[PRIORITIES.length - 1].firstResponseHours,
    );
    expect(STATUSES).toHaveLength(4);
  });
});

describe("slaState", () => {
  test("breaches once the window passes with no reply", () => {
    // Urgent allows 1h; this one has been waiting 3.
    expect(slaState(ticket({ priority: "urgent", createdAt: hoursAgo(3) }), NOW)).toBe(
      "breached",
    );
  });

  test("still due inside the window", () => {
    expect(
      slaState(ticket({ priority: "normal", createdAt: hoursAgo(3) }), NOW),
    ).toBe("due");
  });

  test("a reply meets the SLA, even a late one", () => {
    // The question is "did we reply", and re-flagging answered work just adds
    // noise to the queue.
    expect(
      slaState(
        ticket({
          priority: "urgent",
          createdAt: hoursAgo(9),
          firstRespondedAt: hoursAgo(2),
        }),
        NOW,
      ),
    ).toBe("met");
  });

  test("terminal tickets have no SLA to chase", () => {
    expect(
      slaState(ticket({ status: "solved", priority: "urgent", createdAt: hoursAgo(50) }), NOW),
    ).toBe("none");
  });

  test("an unparseable date is 'none', not a false breach", () => {
    expect(slaState(ticket({ createdAt: "nonsense" }), NOW)).toBe("none");
  });
});

describe("minutesToBreach", () => {
  test("positive while there's time left", () => {
    expect(minutesToBreach(ticket({ priority: "high", createdAt: hoursAgo(1) }), NOW)).toBe(
      180,
    );
  });

  test("negative once overdue", () => {
    expect(
      minutesToBreach(ticket({ priority: "urgent", createdAt: hoursAgo(3) }), NOW),
    ).toBe(-120);
  });

  test("null when answered or closed", () => {
    expect(
      minutesToBreach(ticket({ firstRespondedAt: hoursAgo(1) }), NOW),
    ).toBeNull();
    expect(minutesToBreach(ticket({ status: "closed" }), NOW)).toBeNull();
  });
});

describe("queueOrder", () => {
  test("breached tickets come first, whatever their priority", () => {
    const rows = [
      ticket({ id: "fresh-urgent", priority: "urgent", createdAt: hoursAgo(0.2) }),
      ticket({ id: "breached-low", priority: "low", createdAt: hoursAgo(100) }),
    ];
    expect(queueOrder(rows, NOW).map((t) => t.id)).toEqual([
      "breached-low",
      "fresh-urgent",
    ]);
  });

  test("then by priority", () => {
    const rows = [
      ticket({ id: "normal", priority: "normal", createdAt: hoursAgo(1) }),
      ticket({ id: "urgent", priority: "urgent", createdAt: hoursAgo(0.5) }),
      ticket({ id: "low", priority: "low", createdAt: hoursAgo(1) }),
    ];
    expect(queueOrder(rows, NOW).map((t) => t.id)).toEqual(["urgent", "normal", "low"]);
  });

  test("oldest first within a priority", () => {
    // Newest-first is how a ticket sits at the bottom of a busy queue for a week.
    const rows = [
      ticket({ id: "newer", createdAt: hoursAgo(1), firstRespondedAt: hoursAgo(1) }),
      ticket({ id: "older", createdAt: hoursAgo(5), firstRespondedAt: hoursAgo(5) }),
    ];
    expect(queueOrder(rows, NOW).map((t) => t.id)).toEqual(["older", "newer"]);
  });

  test("does not mutate its input", () => {
    const rows = [ticket({ id: "a" }), ticket({ id: "b", priority: "urgent" })];
    const before = rows.map((t) => t.id);
    queueOrder(rows, NOW);
    expect(rows.map((t) => t.id)).toEqual(before);
  });
});

describe("counts", () => {
  test("tallies statuses, unassigned, and breaches", () => {
    const rows = [
      ticket({ id: "1", status: "open", assigneeId: null, priority: "urgent", createdAt: hoursAgo(5) }),
      ticket({ id: "2", status: "open", assigneeId: "u1", firstRespondedAt: hoursAgo(1) }),
      ticket({ id: "3", status: "pending", assigneeId: "u1", firstRespondedAt: hoursAgo(1) }),
      ticket({ id: "4", status: "solved", assigneeId: "u1" }),
      ticket({ id: "5", status: "closed", assigneeId: "u1" }),
    ];
    expect(counts(rows, NOW)).toEqual({
      open: 2,
      pending: 1,
      solved: 1,
      closed: 1,
      unassigned: 1,
      breached: 1,
    });
  });

  test("a solved ticket with no assignee isn't 'unassigned' work", () => {
    expect(counts([ticket({ status: "solved", assigneeId: null })], NOW).unassigned).toBe(0);
  });
});

describe("applyFilter", () => {
  const rows = [
    ticket({ id: "1", status: "open", priority: "urgent", assigneeId: "u1" }),
    ticket({ id: "2", status: "pending", priority: "low", assigneeId: null }),
    ticket({ id: "3", status: "open", priority: "low", assigneeId: "u2" }),
  ];

  test("an empty filter passes everything", () => {
    expect(applyFilter(rows, {})).toHaveLength(3);
  });

  test("narrows by status, priority, and assignee", () => {
    expect(applyFilter(rows, { status: "open" }).map((t) => t.id)).toEqual(["1", "3"]);
    expect(applyFilter(rows, { priority: "low" }).map((t) => t.id)).toEqual(["2", "3"]);
    expect(applyFilter(rows, { assigneeId: "u2" }).map((t) => t.id)).toEqual(["3"]);
  });

  test("unassigned means no agent at all", () => {
    expect(applyFilter(rows, { unassigned: true }).map((t) => t.id)).toEqual(["2"]);
  });

  test("filters combine", () => {
    expect(applyFilter(rows, { status: "open", priority: "low" }).map((t) => t.id)).toEqual(
      ["3"],
    );
  });
});

describe("ticketNumber", () => {
  test("is stable for a given ticket", () => {
    expect(ticketNumber("abc", "2026-01-01")).toBe(ticketNumber("abc", "2026-01-01"));
  });

  test("looks like a ticket number", () => {
    expect(ticketNumber("abc", "2026-01-01")).toMatch(/^#\d{4}$/);
  });
});
