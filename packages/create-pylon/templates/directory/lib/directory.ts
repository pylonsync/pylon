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

// A tool's mark in the list: its initials on a tinted square. The tint comes
// from a hash of the name, so a tool keeps its color across reloads and
// servers without storing anything. Swap in real logos by adding a `logoUrl`
// field if your directory has them.
const MARK_TONES = [
  { bg: "#e0e7ff", fg: "#3730a3" },
  { bg: "#dcfce7", fg: "#166534" },
  { bg: "#fef3c7", fg: "#92400e" },
  { bg: "#fce7f3", fg: "#9d174d" },
  { bg: "#e0f2fe", fg: "#075985" },
  { bg: "#ede9fe", fg: "#5b21b6" },
  { bg: "#ffedd5", fg: "#9a3412" },
  { bg: "#f1f5f9", fg: "#334155" },
] as const;

export function monogram(name: string): { letters: string; bg: string; fg: string } {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters =
    words.length >= 2
      ? (words[0][0] + words[1][0]).toUpperCase()
      : (words[0] ?? "?").slice(0, 2).replace(/^./, (c) => c.toUpperCase());
  let h = 0;
  for (const ch of name.trim().toLowerCase()) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  const tone = MARK_TONES[h % MARK_TONES.length];
  return { letters, bg: tone.bg, fg: tone.fg };
}
