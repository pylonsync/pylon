// Shared directory types. The Submission row is what the owner dashboard sees
// (with PII); the client imports only the type, never server code.

// A public listing as it syncs to the browse grid (no PII).
export interface ListingRow {
  id: string;
  name: string;
  tagline: string;
  url: string;
  category: string;
  tags?: string | null;
  description?: string | null;
  votes: number;
  featured: boolean;
  createdAt: string;
}

export interface SubmissionRow {
  id: string;
  submitterName: string;
  submitterEmail: string;
  name: string;
  tagline: string;
  url: string;
  category: string;
  tags?: string | null;
  description?: string | null;
  status: string; // "new" | "approved" | "rejected"
  createdAt: string;
}

// submissionsForOwner returns a discriminated result rather than throwing on a
// non-owner (a query has no `ctx.error`; a bare throw becomes a stripped
// HANDLER_ERROR). A non-owner gets `{ authorized: false }` and NO data.
export type OwnerSubmissionsResult =
  | { authorized: true; submissions: SubmissionRow[] }
  | { authorized: false };

// Split a comma-separated tag string into trimmed chips.
export function parseTags(tags?: string | null): string[] {
  return (tags ?? "")
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);
}
