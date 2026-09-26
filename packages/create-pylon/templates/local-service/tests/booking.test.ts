import { describe, expect, test } from "bun:test";
import { demoBookings, demoDataEnabled } from "../lib/demo";
import { clock, directionsUrl, hoursRows } from "../lib/hours";
import { rangesOverlap, slotsForDay, weekdayOf } from "../lib/slots";
import { siteConfig } from "../lib/site.config";

describe("slotsForDay state", () => {
  const base = {
    dayISODate: "2026-09-29",
    open: "09:00",
    close: "11:00",
    slotMinutes: 30,
    durationMin: 30,
    leadTimeHours: 0,
  };
  test("marks booked times taken and early times past", () => {
    const nine = new Date(2026, 8, 29, 9, 0).getTime();
    const slots = slotsForDay({
      ...base,
      nowMs: nine + 31 * 60_000,
      busy: [
        {
          startsAt: new Date(nine + 60 * 60_000).toISOString(),
          endsAt: new Date(nine + 90 * 60_000).toISOString(),
        },
      ],
    });
    expect(slots.map((s) => s.state)).toEqual(["past", "past", "taken", "open"]);
    expect(slots.filter((s) => s.available)).toHaveLength(1);
  });
});

describe("hoursRows", () => {
  test("groups days with the same hours and lists closed days", () => {
    expect(hoursRows(siteConfig.booking.hours)).toEqual([
      { days: "Mon", time: "Closed", closed: true },
      { days: "Tue–Wed", time: "9 AM–6 PM", closed: false },
      { days: "Thu–Fri", time: "9 AM–7 PM", closed: false },
      { days: "Sat", time: "9 AM–4 PM", closed: false },
      { days: "Sun", time: "Closed", closed: true },
    ]);
  });
  test("clock formats noon, midnight, and half hours", () => {
    expect(clock("12:00")).toBe("12 PM");
    expect(clock("00:00")).toBe("12 AM");
    expect(clock("17:30")).toBe("5:30 PM");
  });
  test("directions link encodes the address", () => {
    expect(directionsUrl("1845 Greenville Ave, Dallas")).toBe(
      "https://www.google.com/maps/dir/?api=1&destination=1845%20Greenville%20Ave%2C%20Dallas",
    );
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

  const now = new Date(2026, 8, 26, 10, 15).getTime(); // a Saturday morning
  const rows = demoBookings(now, siteConfig);

  test("every booking sits inside opening hours on an open day", () => {
    expect(rows.length).toBeGreaterThan(40);
    for (const b of rows) {
      const start = new Date(b.startsAt);
      const key = `${start.getFullYear()}-${String(start.getMonth() + 1).padStart(2, "0")}-${String(start.getDate()).padStart(2, "0")}`;
      const hours = siteConfig.booking.hours[weekdayOf(key)];
      expect(hours).not.toBeNull();
      const [ch, cm] = hours!.close.split(":").map(Number);
      const close = new Date(start);
      close.setHours(ch, cm, 0, 0);
      expect(Date.parse(b.endsAt)).toBeLessThanOrEqual(close.getTime());
    }
  });

  test("active bookings never overlap, so the grid and the dashboard agree", () => {
    const active = rows.filter((b) => b.status !== "cancelled");
    for (let i = 0; i < active.length; i++) {
      for (let j = i + 1; j < active.length; j++) {
        const a = active[i];
        const b = active[j];
        expect(
          rangesOverlap(Date.parse(a.startsAt), Date.parse(a.endsAt), Date.parse(b.startsAt), Date.parse(b.endsAt)),
        ).toBe(false);
      }
    }
  });

  test("past bookings are confirmed; upcoming ones include some to confirm", () => {
    const past = rows.filter((b) => Date.parse(b.endsAt) <= now);
    const future = rows.filter((b) => Date.parse(b.endsAt) > now);
    expect(past.every((b) => b.status === "confirmed")).toBe(true);
    expect(future.some((b) => b.status === "pending")).toBe(true);
    expect(rows.every((b) => Date.parse(b.createdAt) < now)).toBe(true);
    // Phone numbers stay in the 555-0100..0199 range reserved for fiction.
    for (const b of rows) if (b.customerPhone) expect(b.customerPhone).toMatch(/^\(214\) 555-01\d{2}$/);
  });
});
