// Shape of a booking row as the owner dashboard sees it, shared by the server
// query (functions/bookingsForOwner.ts) and the client view
// (app/dashboard/dashboard-client.tsx). The client imports only the type, never
// server code.

export interface BookingRow {
  id: string;
  serviceSlug: string;
  startsAt: string;
  endsAt: string;
  customerName: string;
  customerEmail: string;
  customerPhone?: string | null;
  status: string; // "pending" | "confirmed" | "cancelled"
  createdAt: string;
}

// bookingsForOwner returns a discriminated result rather than throwing on a
// non-owner: a query has no `ctx.error`, and a bare throw reaches the client as
// a stripped HANDLER_ERROR. A non-owner gets `{ authorized: false }` and NO
// booking data.
export type OwnerBookingsResult =
  | { authorized: true; bookings: BookingRow[] }
  | { authorized: false };
