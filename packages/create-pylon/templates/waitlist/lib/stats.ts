// Shape of the owner dashboard's data, shared by the server query
// (functions/waitlistStats.ts) that produces it and the client view
// (app/dashboard/dashboard-client.tsx) that renders it. Keeping it here means
// the client never imports server code — just the type.

export interface SignupRow {
  id: string;
  email: string;
  createdAt: string;
}

export interface WaitlistStatsData {
  total: number;
  today: number;
  last7: number;
  /** Last 30 days, ascending — continuous (zero-filled) for the chart. */
  daily: { date: string; count: number }[];
  /** Every signup, newest first — powers the list + CSV export. */
  signups: SignupRow[];
}

// waitlistStats returns a DISCRIMINATED result rather than throwing on a
// non-owner: a query handler has no `ctx.error`, and a plain `throw` reaches
// the client as a generic, message-stripped HANDLER_ERROR — useless for telling
// "you're not the owner" apart from a real failure. So a non-owner gets
// `{ authorized: false }` (and crucially, NO signup data); the owner gets the
// stats. The client switches on `authorized`.
export type WaitlistStatsResult =
  | ({ authorized: true } & WaitlistStatsData)
  | { authorized: false };

/* ------------------------- public live counter ------------------------- */

// The public counter shows the Signup table's row count, read from the
// PII-free WaitlistStat row. `importedCount` is `siteConfig.counter.importedCount`:
// signups collected before this site existed (an old form, a spreadsheet). It is
// 0 by default, so the page shows only rows in the table.
export function publicCount(
  statRows: ReadonlyArray<{ count: number }>,
  importedCount = 0,
): number {
  const real = statRows.length > 0 ? Math.max(0, Math.floor(statRows[0].count)) : 0;
  const imported = Number.isFinite(importedCount) ? Math.max(0, Math.floor(importedCount)) : 0;
  return real + imported;
}

// The subset of `ctx.db.unsafe` the recount needs. Typed structurally so the
// helper runs against the real handle and a test double.
export interface StatDb {
  list(entity: string): Promise<unknown[]>;
  insert(entity: string, data: Record<string, unknown>): Promise<unknown>;
  update(entity: string, id: string, data: Record<string, unknown>): Promise<unknown>;
}

// Write the Signup row count into the single WaitlistStat row, creating it if
// missing. Every writer of Signup calls this in the same transaction, so the
// public number is always the table's count.
export async function syncWaitlistStat(db: StatDb): Promise<number> {
  const total = (await db.list("Signup")).length;
  const stat = (await db.list("WaitlistStat"))[0] as { id: string } | undefined;
  const now = new Date().toISOString();
  if (stat) await db.update("WaitlistStat", stat.id, { count: total, updatedAt: now });
  else await db.insert("WaitlistStat", { count: total, updatedAt: now });
  return total;
}
