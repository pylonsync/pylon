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
