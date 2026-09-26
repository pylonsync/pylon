import { describe, expect, test } from "bun:test";
import { publicCount, syncWaitlistStat, type StatDb } from "../lib/stats";
import { demoDataEnabled, demoSignups } from "../lib/demo";
import { launchLabel } from "../lib/launch";
import { siteConfig } from "../lib/site.config";

describe("publicCount", () => {
  test("is the table's count when nothing is imported", () => {
    expect(publicCount([{ count: 3 }])).toBe(3);
    expect(publicCount([])).toBe(0);
  });
  test("adds only an explicit, non-negative imported count", () => {
    expect(publicCount([{ count: 3 }], 40)).toBe(43);
    expect(publicCount([{ count: 3 }], -5)).toBe(3);
    expect(publicCount([{ count: 3 }], Number.NaN)).toBe(3);
  });
  test("the shipped config adds nothing to the real count", () => {
    expect(siteConfig.counter.importedCount).toBe(0);
  });
});

// An in-memory stand-in for ctx.db.unsafe.
function fakeDb(signups: number): StatDb & { rows: Record<string, Record<string, unknown>[]> } {
  const rows: Record<string, Record<string, unknown>[]> = {
    Signup: Array.from({ length: signups }, (_, i) => ({ id: `s${i}` })),
    WaitlistStat: [],
  };
  return {
    rows,
    async list(entity) {
      return rows[entity] ?? [];
    },
    async insert(entity, data) {
      const id = `${entity}-${rows[entity].length}`;
      rows[entity].push({ id, ...data });
      return id;
    },
    async update(entity, id, data) {
      const row = rows[entity].find((r) => r.id === id);
      if (row) Object.assign(row, data);
      return id;
    },
  };
}

describe("syncWaitlistStat", () => {
  test("creates the stat row with the Signup count", async () => {
    const db = fakeDb(5);
    expect(await syncWaitlistStat(db)).toBe(5);
    expect(db.rows.WaitlistStat).toHaveLength(1);
    expect(db.rows.WaitlistStat[0].count).toBe(5);
  });
  test("recounts into the existing row instead of adding one", async () => {
    const db = fakeDb(5);
    await syncWaitlistStat(db);
    db.rows.Signup.push({ id: "late" });
    await syncWaitlistStat(db);
    expect(db.rows.WaitlistStat).toHaveLength(1);
    expect(db.rows.WaitlistStat[0].count).toBe(6);
  });
});

describe("demoDataEnabled", () => {
  test("needs a `pylon dev` process when PYLON_DEMO_DATA is unset", () => {
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app" })).toBe(true);
    // The Docker base image turns dev mode on by default; without `pylon dev` it must not seed.
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true" })).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true", PYLON_DEV_WATCH_DIR: "/src/app" })).toBe(true);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "false" })).toBe(false);
    expect(demoDataEnabled({})).toBe(false);
  });
  test("an explicit PYLON_DEMO_DATA wins", () => {
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app", PYLON_DEMO_DATA: "0" })).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "false", PYLON_DEMO_DATA: "1" })).toBe(true);
  });
});

describe("demoSignups", () => {
  const now = Date.parse("2026-09-26T15:00:00Z");
  const rows = demoSignups(now);
  test("unique emails, all in the past 30 days", () => {
    expect(new Set(rows.map((r) => r.email)).size).toBe(rows.length);
    for (const r of rows) {
      const t = Date.parse(r.createdAt);
      expect(t).toBeLessThan(now);
      expect(t).toBeGreaterThan(now - 31 * 86_400_000);
    }
  });
  test("enough rows to fill the chart, stable for a seed", () => {
    expect(rows.length).toBeGreaterThan(120);
    expect(demoSignups(now)).toEqual(rows);
  });
});

describe("launchLabel", () => {
  const now = Date.parse("2026-09-26T12:00:00Z");
  test("names the month before the date", () => {
    expect(launchLabel("2027-03-02", now)).toBe("Public launch: March 2027");
  });
  test("never shows a date that has passed", () => {
    expect(launchLabel("2026-09-01", now)).toBe("Invites are going out now");
  });
  test("rejects a malformed date", () => {
    expect(launchLabel("fall 2026", now)).toBeNull();
    expect(launchLabel("2026-02-31", now)).toBeNull();
  });
});
