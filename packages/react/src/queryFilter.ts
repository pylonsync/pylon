// Where-filter evaluation for queries answered from the local replica.
//
// The server answers the same filter in SQL, so these comparisons follow
// the server's:
//
//   - Numbers compare numerically.
//   - Strings compare by code unit, like SQLite's BINARY collation (not by
//     locale).
//   - Two RFC 3339 timestamps compare as instants. The server stores every
//     `datetime` value in one canonical UTC form and canonicalizes a filter
//     operand the same way, so `occurredAt: { $gte: "2026-10-01T00:00:00Z" }`
//     matches by time whatever offset or precision either side was written
//     with.
//   - A missing or null value matches no range (SQL `NULL > x` is not true).
//
// An operator that cannot apply (a number against a string, an unknown
// operator) matches nothing, as on the server, and warns once in dev, so
// a filter that silently empties a screen shows up in the console.

const RFC3339 = /^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;

/** Epoch milliseconds of an RFC 3339 timestamp, or null. */
function instant(value: string): number | null {
  if (!RFC3339.test(value)) return null;
  const ms = Date.parse(value);
  return Number.isNaN(ms) ? null : ms;
}

/**
 * Order two values the way the server's SQL does: negative, zero or
 * positive; `null` when they cannot be compared (different types, or an
 * object). Null and undefined are not handled here.
 */
export function compareValues(a: unknown, b: unknown): number | null {
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "string" && typeof b === "string") {
    const ta = instant(a);
    const tb = ta === null ? null : instant(b);
    if (ta !== null && tb !== null) return ta - tb;
    return a < b ? -1 : a > b ? 1 : 0;
  }
  if (typeof a === "boolean" && typeof b === "boolean") return Number(a) - Number(b);
  return null;
}

const warned = new Set<string>();

function warnOnce(field: string, op: string, reason: string): void {
  if (typeof process !== "undefined" && process.env?.NODE_ENV === "production") return;
  const key = `${field}:${op}:${reason}`;
  if (warned.has(key)) return;
  warned.add(key);
  // eslint-disable-next-line no-console
  console.warn(`[pylon] where.${field}.${op} cannot apply (${reason}); it matches no rows.`);
}

type Row = Record<string, unknown>;

const RANGE: Record<string, (c: number) => boolean> = {
  $gt: (c) => c > 0,
  $gte: (c) => c >= 0,
  $lt: (c) => c < 0,
  $lte: (c) => c <= 0,
};

/** A predicate for `where` (keys other than `$order` and `$limit`). */
export function compileWhere(where: Record<string, unknown>): (row: Row) => boolean {
  const fields = Object.entries(where)
    .filter(([key]) => key !== "$order" && key !== "$limit")
    .map(([key, val]) => ({
      key,
      val,
      operators:
        val !== null && typeof val === "object" && !Array.isArray(val)
          ? Object.entries(val as Record<string, unknown>).map(([op, opVal]) => ({
              op,
              opVal,
              members: op === "$in" && Array.isArray(opVal) ? new Set(opVal) : undefined,
            }))
          : undefined,
    }));
  return (row) => {
    for (const { key, val, operators } of fields) {
      const rowVal = row[key];
      if (!operators) {
        if (rowVal !== val) return false;
        continue;
      }
      for (const { op, opVal, members } of operators) {
        const range = RANGE[op];
        if (range) {
          if (rowVal === null || rowVal === undefined) return false;
          if (opVal === null || opVal === undefined) {
            warnOnce(key, op, "the operand is null");
            return false;
          }
          const c = compareValues(rowVal, opVal);
          if (c === null) {
            warnOnce(key, op, `a ${typeof rowVal} value against a ${typeof opVal} operand`);
            return false;
          }
          if (!range(c)) return false;
          continue;
        }
        switch (op) {
          case "$not":
            if (rowVal === opVal) return false;
            break;
          case "$like":
            if (!(typeof rowVal === "string" && typeof opVal === "string" && rowVal.includes(opVal)))
              return false;
            break;
          case "$in":
            if (!members?.has(rowVal)) return false;
            break;
          default:
            warnOnce(key, op, "unknown operator");
            return false;
        }
      }
    }
    return true;
  };
}

/** Sort comparator for `$order` / `orderBy`: nulls last ascending, the
 *  server's value order otherwise. */
export function compareForOrder(a: unknown, b: unknown, dir: "asc" | "desc"): number {
  const mult = dir === "desc" ? -1 : 1;
  if (a === b) return 0;
  if (a === undefined || a === null) return mult;
  if (b === undefined || b === null) return -mult;
  const c = compareValues(a, b);
  if (c !== null) return Math.sign(c) * mult;
  const sa = String(a);
  const sb = String(b);
  return (sa < sb ? -1 : sa > sb ? 1 : 0) * mult;
}
