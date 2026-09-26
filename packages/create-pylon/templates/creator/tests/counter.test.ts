import { describe, expect, test } from "bun:test";
import { publicCount, syncSubscriberCount, type CountDb } from "../lib/stats";
import { demoDataEnabled, demoSubscribers } from "../lib/demo";
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
    expect(siteConfig.newsletter.importedCount).toBe(0);
  });
});

// An in-memory stand-in for ctx.db.unsafe.
function fakeDb(signups: number): CountDb & { rows: Record<string, Record<string, unknown>[]> } {
  const rows: Record<string, Record<string, unknown>[]> = {
    Subscriber: Array.from({ length: signups }, (_, i) => ({ id: `s${i}` })),
    SubscriberCount: [],
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

describe("syncSubscriberCount", () => {
  test("creates the stat row with the Subscriber count", async () => {
    const db = fakeDb(5);
    expect(await syncSubscriberCount(db)).toBe(5);
    expect(db.rows.SubscriberCount).toHaveLength(1);
    expect(db.rows.SubscriberCount[0].count).toBe(5);
  });
  test("recounts into the existing row instead of adding one", async () => {
    const db = fakeDb(5);
    await syncSubscriberCount(db);
    db.rows.Subscriber.push({ id: "late" });
    await syncSubscriberCount(db);
    expect(db.rows.SubscriberCount).toHaveLength(1);
    expect(db.rows.SubscriberCount[0].count).toBe(6);
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

describe("demoSubscribers", () => {
  const now = Date.parse("2026-09-26T15:00:00Z");
  const rows = demoSubscribers(now);
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
    expect(demoSubscribers(now)).toEqual(rows);
  });
});
