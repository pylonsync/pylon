// Weekly hours as display rows, from `siteConfig.booking.hours` (the same
// hours the booking grid uses), so the location block cannot disagree with the
// grid. Consecutive days with the same hours share a row: "Tue–Wed 9 AM–6 PM".

import type { DayHours } from "./site.config";

const DAY = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
// Rows run Monday to Sunday, the way a shop door lists them.
const ORDER = [1, 2, 3, 4, 5, 6, 0];

export interface HoursRow {
  days: string;
  time: string; // "9 AM–6 PM" or "Closed"
  closed: boolean;
}

/** "09:00" → "9 AM", "18:30" → "6:30 PM". */
export function clock(hhmm: string): string {
  const [h, m] = hhmm.split(":").map(Number);
  const suffix = h >= 12 ? "PM" : "AM";
  const h12 = h % 12 === 0 ? 12 : h % 12;
  return m ? `${h12}:${String(m).padStart(2, "0")} ${suffix}` : `${h12} ${suffix}`;
}

export function hoursRows(hours: Record<number, DayHours>): HoursRow[] {
  const rows: { first: number; last: number; time: string }[] = [];
  for (const d of ORDER) {
    const h = hours[d];
    const time = h ? `${clock(h.open)}–${clock(h.close)}` : "Closed";
    const prev = rows[rows.length - 1];
    if (prev && prev.time === time) prev.last = d;
    else rows.push({ first: d, last: d, time });
  }
  return rows.map((r) => ({
    days: r.first === r.last ? DAY[r.first] : `${DAY[r.first]}–${DAY[r.last]}`,
    time: r.time,
    closed: r.time === "Closed",
  }));
}

/** A Google Maps directions link for a street address. Needs no API key. */
export function directionsUrl(address: string): string {
  return `https://www.google.com/maps/dir/?api=1&destination=${encodeURIComponent(address)}`;
}
