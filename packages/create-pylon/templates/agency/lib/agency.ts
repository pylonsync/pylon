// Shared agency types. The Inquiry row is what the owner dashboard sees (with
// PII); the client imports only the type, never server code.

export interface InquiryRow {
  id: string;
  name: string;
  email: string;
  company?: string | null;
  projectType?: string | null;
  budget?: string | null;
  message?: string | null;
  status: string; // "new" | "booked" | "declined"
  createdAt: string;
}

// inquiriesForOwner returns a discriminated result rather than throwing on a
// non-owner (a query has no `ctx.error`; a bare throw becomes a stripped
// HANDLER_ERROR). A non-owner gets `{ authorized: false }` and NO data.
export type OwnerInquiriesResult =
  | { authorized: true; inquiries: InquiryRow[] }
  | { authorized: false };

// The public, PII-free capacity the landing page reads live.
export interface CapacityData {
  label: string; // booking window, e.g. "Q3 2026"
  openSlots: number;
}
