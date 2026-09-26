import { describe, expect, test } from "bun:test";
import { demoDataEnabled, demoReservations } from "../lib/demo";
import { clock, directionsUrl, hoursRows } from "../lib/hours";
import { siteConfig } from "../lib/site.config";
import { weekdayOf } from "../lib/slots";

describe("hoursRows", () => {
  test("groups days with the same service hours", () => {
    expect(hoursRows(siteConfig.reservations.hours)).toEqual([
      { days: "Mon–Tue", time: "Closed", closed: true },
      { days: "Wed–Thu", time: "5 PM–9:30 PM", closed: false },
      { days: "Fri–Sat", time: "5 PM–10 PM", closed: false },
      { days: "Sun", time: "5 PM–9 PM", closed: false },
    ]);
  });
  test("clock and directions link", () => {
    expect(clock("21:30")).toBe("9:30 PM");
    expect(directionsUrl("412 N Bishop Ave")).toContain("destination=412%20N%20Bishop%20Ave");
  });
});

describe("demo reservations", () => {
  test("only under `pylon dev` unless PYLON_DEMO_DATA says otherwise", () => {
    expect(demoDataEnabled({ PYLON_DEV_MODE: "1", PYLON_DEV_WATCH_DIR: "/src/app" })).toBe(true);
    // The Docker base image turns dev mode on by default; without `pylon dev` it must not seed.
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true" })).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "false" })).toBe(false);
    expect(demoDataEnabled({})).toBe(false);
    expect(demoDataEnabled({ PYLON_DEV_MODE: "true", PYLON_DEV_WATCH_DIR: "/src/app", PYLON_DEMO_DATA: "off" })).toBe(false);
  });

  const now = new Date(2026, 8, 26, 14, 0).getTime();
  const rows = demoReservations(now, siteConfig);
  const cfg = siteConfig.reservations;

  test("never books more active tables than a seating has", () => {
    const perSeat = new Map<string, number>();
    for (const r of rows) {
      if (r.status === "cancelled") continue;
      perSeat.set(r.startsAt, (perSeat.get(r.startsAt) ?? 0) + 1);
    }
    expect(Math.max(...perSeat.values())).toBeLessThanOrEqual(cfg.tablesPerSlot);
    // Some weekend seatings fill up, so the calendar shows "Full".
    expect([...perSeat.values()].some((n) => n === cfg.tablesPerSlot)).toBe(true);
  });

  test("only on open days, within the seating window, with valid parties", () => {
    for (const r of rows) {
      const d = new Date(r.startsAt);
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
      const hours = cfg.hours[weekdayOf(key)];
      expect(hours).not.toBeNull();
      const hhmm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
      expect(hhmm >= hours!.open && hhmm <= hours!.close).toBe(true);
      expect(r.partySize).toBeGreaterThanOrEqual(1);
      expect(r.partySize).toBeLessThanOrEqual(cfg.maxPartySize);
      expect(Date.parse(r.createdAt)).toBeLessThan(now);
      if (r.customerPhone) expect(r.customerPhone).toMatch(/^\(214\) 555-01\d{2}$/);
    }
  });

  test("past tables are confirmed; upcoming ones include some to confirm", () => {
    const past = rows.filter((r) => Date.parse(r.startsAt) <= now && r.status !== "cancelled");
    expect(past.every((r) => r.status === "confirmed")).toBe(true);
    expect(rows.some((r) => Date.parse(r.startsAt) > now && r.status === "pending")).toBe(true);
  });
});
