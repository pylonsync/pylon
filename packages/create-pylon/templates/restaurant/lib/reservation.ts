// Shape of a reservation row as the owner dashboard sees it, shared by the
// server query (functions/reservationsForOwner.ts) and the client view. The
// client imports only the type, never server code.

export interface ReservationRow {
  id: string;
  startsAt: string;
  partySize: number;
  customerName: string;
  customerEmail: string;
  customerPhone?: string | null;
  notes?: string | null;
  status: string; // "pending" | "confirmed" | "cancelled"
  createdAt: string;
}

// reservationsForOwner returns a discriminated result rather than throwing on a
// non-owner: a query has no `ctx.error`, and a bare throw reaches the client as
// a stripped HANDLER_ERROR. A non-owner gets `{ authorized: false }` and NO data.
export type OwnerReservationsResult =
  | { authorized: true; reservations: ReservationRow[] }
  | { authorized: false };
