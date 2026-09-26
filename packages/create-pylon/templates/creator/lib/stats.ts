// Shape of the owner dashboard's data, shared by the server query
// (functions/subscriberStats.ts) that produces it and the client view
// (app/dashboard/dashboard-client.tsx) that renders it. Keeping it here means
// the client never imports server code — just the type.

export interface SubscriberRow {
  id: string;
  email: string;
  createdAt: string;
}

export interface SubscriberStatsData {
  total: number;
  today: number;
  last7: number;
  /** Last 30 days, ascending — continuous (zero-filled) for the chart. */
  daily: { date: string; count: number }[];
  /** Every subscriber, newest first — powers the list + CSV export. */
  subscribers: SubscriberRow[];
}

// subscriberStats returns a DISCRIMINATED result rather than throwing on a
// non-owner: a query handler has no `ctx.error`, and a plain `throw` reaches
// the client as a generic, message-stripped HANDLER_ERROR — useless for telling
// "you're not the owner" apart from a real failure. So a non-owner gets
// `{ authorized: false }` (and crucially, NO subscriber data); the owner gets the
// stats. The client switches on `authorized`.
export type SubscriberStatsResult =
  | ({ authorized: true } & SubscriberStatsData)
  | { authorized: false };

/* ------------------------- public live counter ------------------------- */

// The public counter shows the Subscriber table's row count, read from the
// PII-free SubscriberCount row. `importedCount` is
// `siteConfig.newsletter.importedCount`: subscribers you are moving over from
// another tool. It is 0 by default, so the page shows only rows in the table.
export function publicCount(
  countRows: ReadonlyArray<{ count: number }>,
  importedCount = 0,
): number {
  const real = countRows.length > 0 ? Math.max(0, Math.floor(countRows[0].count)) : 0;
  const imported = Number.isFinite(importedCount) ? Math.max(0, Math.floor(importedCount)) : 0;
  return real + imported;
}

// The subset of `ctx.db.unsafe` the recount needs. Typed structurally so the
// helper runs against the real handle and a test double.
export interface CountDb {
  list(entity: string): Promise<unknown[]>;
  insert(entity: string, data: Record<string, unknown>): Promise<unknown>;
  update(entity: string, id: string, data: Record<string, unknown>): Promise<unknown>;
}

// Write the Subscriber row count into the single SubscriberCount row, creating
// it if missing. Every writer of Subscriber calls this in the same transaction,
// so the public number is always the table's count.
export async function syncSubscriberCount(db: CountDb): Promise<number> {
  const total = (await db.list("Subscriber")).length;
  const row = (await db.list("SubscriberCount"))[0] as { id: string } | undefined;
  const now = new Date().toISOString();
  if (row) await db.update("SubscriberCount", row.id, { count: total, updatedAt: now });
  else await db.insert("SubscriberCount", { count: total, updatedAt: now });
  return total;
}
