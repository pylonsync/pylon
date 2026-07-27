import { describe, expect, test } from "bun:test";
import {
  BOARD_STAGES,
  PIPELINE,
  accentIndex,
  daysUntil,
  groupByStage,
  initials,
  isOpen,
  isValidStage,
  metrics,
  money,
  nextStage,
  percent,
  previousStage,
  relativeTime,
  sumValue,
  type Deal,
} from "../lib/pipeline";

const deal = (over: Partial<Deal> = {}): Deal => ({
  id: "d1",
  title: "Deal",
  value: 1000,
  stage: "lead",
  ...over,
});

describe("stages", () => {
  test("the board hides Lost but the model keeps it", () => {
    // Lost deals still count against the win rate; they just don't deserve a
    // column competing for attention with live work.
    expect(BOARD_STAGES.map((s) => s.id)).toEqual([
      "lead",
      "qualified",
      "proposal",
      "won",
    ]);
    expect(PIPELINE.some((s) => s.id === "lost")).toBe(true);
  });

  test("open means not closed", () => {
    expect(isOpen({ stage: "proposal" })).toBe(true);
    expect(isOpen({ stage: "won" })).toBe(false);
    expect(isOpen({ stage: "lost" })).toBe(false);
  });

  test("advance and retreat stop at the ends", () => {
    expect(nextStage("lead")).toBe("qualified");
    expect(nextStage("proposal")).toBe("won");
    expect(nextStage("won")).toBeNull();
    expect(previousStage("qualified")).toBe("lead");
    expect(previousStage("lead")).toBeNull();
  });

  test("an unknown stage is rejected, not coerced", () => {
    expect(isValidStage("proposal")).toBe(true);
    expect(isValidStage("Proposal")).toBe(false);
    expect(isValidStage("negotiation")).toBe(false);
    expect(nextStage("nonsense")).toBeNull();
  });
});

describe("groupByStage", () => {
  test("puts each deal in its column, newest first", () => {
    const columns = groupByStage([
      deal({ id: "a", stage: "lead", createdAt: "2026-01-01" }),
      deal({ id: "b", stage: "lead", createdAt: "2026-03-01" }),
      deal({ id: "c", stage: "won" }),
    ]);
    const lead = columns.find((c) => c.stage.id === "lead");
    expect(lead?.deals.map((d) => d.id)).toEqual(["b", "a"]);
    expect(columns.find((c) => c.stage.id === "won")?.deals).toHaveLength(1);
  });

  test("totals each column", () => {
    const columns = groupByStage([
      deal({ id: "a", stage: "lead", value: 1000 }),
      deal({ id: "b", stage: "lead", value: 250 }),
    ]);
    expect(columns.find((c) => c.stage.id === "lead")?.total).toBe(1250);
  });

  test("a deal with an unknown stage is dropped, not filed under Lead", () => {
    // Silently reclassifying it would hide a typo behind a plausible board.
    const columns = groupByStage([deal({ stage: "negotiation" })]);
    expect(columns.every((c) => c.deals.length === 0)).toBe(true);
  });

  test("missing or non-numeric values count as zero", () => {
    expect(sumValue([deal({ value: null }), deal({ value: undefined })])).toBe(0);
  });
});

describe("metrics", () => {
  const book = [
    deal({ id: "1", stage: "lead", value: 10_000 }),
    deal({ id: "2", stage: "proposal", value: 20_000 }),
    deal({ id: "3", stage: "won", value: 30_000 }),
    deal({ id: "4", stage: "lost", value: 40_000 }),
  ];

  test("open excludes closed deals", () => {
    expect(metrics(book).open).toBe(30_000);
    expect(metrics(book).openCount).toBe(2);
  });

  test("weighted applies each stage's probability", () => {
    // 10k * 0.1 + 20k * 0.6
    expect(metrics(book).weighted).toBe(13_000);
  });

  test("won and lost are tracked separately", () => {
    expect(metrics(book).won).toBe(30_000);
    expect(metrics(book).lost).toBe(40_000);
  });

  test("win rate is over CLOSED deals only", () => {
    expect(metrics(book).winRate).toBe(0.5);
  });

  test("win rate is null before anything closes, not 0%", () => {
    // 0% and "nothing has closed yet" mean very different things to a sales lead.
    expect(metrics([deal({ stage: "lead" })]).winRate).toBeNull();
    expect(percent(null)).toBe("—");
  });

  test("an empty book is all zeroes, not NaN", () => {
    expect(metrics([])).toMatchObject({ open: 0, weighted: 0, won: 0, winRate: null });
  });
});

describe("money", () => {
  test("compacts so columns line up in a narrow card", () => {
    expect(money(1_240_000)).toBe("$1.2M");
    expect(money(2_000_000)).toBe("$2M");
    expect(money(48_000)).toBe("$48K");
    expect(money(9_500)).toBe("$9,500");
    expect(money(0)).toBe("$0");
  });

  test("handles nothing and negatives", () => {
    expect(money(null)).toBe("$0");
    expect(money(undefined)).toBe("$0");
    expect(money(-1500)).toBe("-$1,500");
  });
});

describe("initials", () => {
  test("takes first and last for a name", () => {
    expect(initials("Dana Whitfield")).toBe("DW");
    expect(initials("Acme Corporation Holdings")).toBe("AH");
  });

  test("uses the local part of an email", () => {
    expect(initials("jordan@example.com")).toBe("JO");
    expect(initials("dana.whitfield@example.com")).toBe("DW");
  });

  test("degrades rather than throwing", () => {
    expect(initials("")).toBe("?");
    expect(initials(null)).toBe("?");
    expect(initials("   ")).toBe("?");
  });
});

describe("accentIndex", () => {
  test("is stable for the same seed", () => {
    expect(accentIndex("Acme")).toBe(accentIndex("Acme"));
  });

  test("stays in range", () => {
    for (const seed of ["", "a", "Northwind Logistics", "🙂"]) {
      const index = accentIndex(seed, 6);
      expect(index).toBeGreaterThanOrEqual(0);
      expect(index).toBeLessThan(6);
    }
  });
});

describe("relativeTime", () => {
  const now = Date.parse("2026-07-27T12:00:00Z");
  const ago = (ms: number) => new Date(now - ms).toISOString();

  test("reads naturally across the ranges", () => {
    expect(relativeTime(ago(10_000), now)).toBe("just now");
    expect(relativeTime(ago(5 * 60_000), now)).toBe("5m ago");
    expect(relativeTime(ago(4 * 3_600_000), now)).toBe("4h ago");
    expect(relativeTime(ago(3 * 86_400_000), now)).toBe("3d ago");
  });

  test("falls back to a date beyond a week", () => {
    expect(relativeTime(ago(30 * 86_400_000), now)).toMatch(/^[A-Z][a-z]{2} \d+$/);
  });

  test("empty for missing or unparseable input", () => {
    expect(relativeTime(null, now)).toBe("");
    expect(relativeTime("not-a-date", now)).toBe("");
  });
});

describe("daysUntil", () => {
  const now = Date.parse("2026-07-27T18:00:00Z");

  test("counts whole days, ignoring the time of day", () => {
    // A deal closing "tomorrow" shouldn't read as 0 just because it's evening.
    expect(daysUntil("2026-07-28T02:00:00Z", now)).toBe(1);
    expect(daysUntil("2026-07-27T23:00:00Z", now)).toBe(0);
  });

  test("negative when overdue", () => {
    expect(daysUntil("2026-07-20T12:00:00Z", now)).toBe(-7);
  });

  test("null when there's no date", () => {
    expect(daysUntil(null, now)).toBeNull();
    expect(daysUntil("nope", now)).toBeNull();
  });
});
