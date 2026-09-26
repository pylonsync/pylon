import { describe, expect, test } from "bun:test";
import { bookingWindow, isWideCard, quarterLabel } from "../lib/agency";
import { DEMO_INQUIRIES, daysAgoDate, demoDataEnabled } from "../lib/demo";
import { siteConfig } from "../lib/site.config";

describe("booking window", () => {
  test("an empty label means the quarter a project could start in", () => {
    expect(quarterLabel(new Date(2026, 8, 26).getTime())).toBe("Q3 2026");
    expect(quarterLabel(new Date(2026, 9, 1).getTime())).toBe("Q4 2026");
    expect(bookingWindow("", new Date(2027, 0, 5).getTime())).toBe("Q1 2027");
    expect(bookingWindow("  ", new Date(2027, 0, 5).getTime())).toBe("Q1 2027");
    // Three weeks from the end of a quarter, the window moves to the next one.
    expect(bookingWindow("", new Date(2026, 8, 5).getTime())).toBe("Q3 2026");
    expect(bookingWindow("", new Date(2026, 8, 26).getTime())).toBe("Q4 2026");
    expect(bookingWindow("", new Date(2026, 11, 20).getTime())).toBe("Q1 2027");
  });
  test("a fixed label wins", () => {
    expect(bookingWindow("January", Date.now())).toBe("January");
  });
  test("the shipped config does not pin a quarter", () => {
    expect(siteConfig.capacity.label).toBe("");
  });
});

describe("work grid", () => {
  test("an odd count widens the first card so no card sits alone on the last row", () => {
    expect([0, 1, 2].map((i) => isWideCard(i, 3))).toEqual([true, false, false]);
    expect([0, 1, 2, 3].map((i) => isWideCard(i, 4))).toEqual([false, false, false, false]);
    expect(isWideCard(0, 1)).toBe(false);
  });
  test("the shipped homepage has three selected case studies, so the rule applies", () => {
    expect(siteConfig.work.items.filter((w) => w.selected).length % 2).toBe(1);
  });
});

describe("demo data", () => {
  test("only under `pylon dev` unless PYLON_DEMO_DATA says otherwise", () => {
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app" })).toBe(true);
    // The Docker base image turns dev mode on by default; without `pylon dev` it must not seed.
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true" })).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "false" })).toBe(false);
    expect(demoDataEnabled({})).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app", PYLON_DEMO_DATA: "0" })).toBe(false);
  });
  test("demo leads use the contact form's own options", () => {
    for (const q of DEMO_INQUIRIES) {
      expect(siteConfig.contact.projectTypes).toContain(q.projectType);
      expect(siteConfig.contact.budgets).toContain(q.budget);
    }
  });
  test("demo invoice dates are relative to today", () => {
    const now = new Date(2026, 8, 26).getTime();
    expect(daysAgoDate(12, now)).toBe("2026-09-14");
    expect(daysAgoDate(-18, now)).toBe("2026-10-14");
    for (const inv of siteConfig.backoffice.invoices) expect(inv.issuedDaysAgo).toBeGreaterThan(0);
  });
});
