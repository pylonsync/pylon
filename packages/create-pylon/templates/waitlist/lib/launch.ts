// The launch line under the hero form, from `siteConfig.hero.launchDate`.
//
// Before the date it names the month ("Public launch: March 2027"). From the
// date on it says invites are going out, so the page never shows a launch date
// that has already passed. The date is read and formatted in UTC, so the
// server render and the browser produce the same text.

/** `launchDate` is "YYYY-MM-DD" (UTC). Returns null for a malformed date. */
export function launchLabel(launchDate: string, nowMs: number): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(launchDate.trim());
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const atMs = Date.UTC(y, mo - 1, d);
  const at = new Date(atMs);
  if (at.getUTCFullYear() !== y || at.getUTCMonth() !== mo - 1 || at.getUTCDate() !== d) return null;
  if (nowMs >= atMs) return "Invites are going out now";
  const month = at.toLocaleDateString("en-US", { month: "long", year: "numeric", timeZone: "UTC" });
  return `Public launch: ${month}`;
}
