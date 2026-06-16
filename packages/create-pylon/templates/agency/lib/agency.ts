// Shared agency types. The Inquiry / Client / Invoice rows are what the owner
// dashboard sees (with PII + money); the client imports only the types, never
// server code. Project is public, so it's read on both the marketing site and
// the dashboard.

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
// HANDLER_ERROR). A non-owner gets `{ authorized: false }` and NO data. The
// other owner-gated queries (clientsForOwner / invoicesForOwner) follow the
// same shape — see OwnerResult below.
export type OwnerInquiriesResult =
  | { authorized: true; inquiries: InquiryRow[] }
  | { authorized: false };

// The public, PII-free capacity the landing page reads live.
export interface CapacityData {
  label: string; // booking window, e.g. "Q3 2026"
  openSlots: number;
}

// A portfolio piece + case study. Public — read on the marketing site AND in
// the dashboard. `selected` features it on the homepage; `published` toggles
// whether the public site shows it (drafts stay owner-only).
export interface ProjectRow {
  id: string;
  title: string;
  slug: string;
  client: string;
  summary: string;
  year?: string | null;
  tags?: string | null; // comma-separated
  selected: boolean;
  published: boolean;
  order: number;
  challenge?: string | null;
  approach?: string | null;
  outcome?: string | null;
  liveUrl?: string | null;
  createdAt: string;
}

// A CRM contact (owner-only — PII).
export interface ClientRow {
  id: string;
  name: string;
  company?: string | null;
  email?: string | null;
  phone?: string | null;
  status: string; // "prospect" | "active" | "past"
  notes?: string | null;
  createdAt: string;
}

// One line on an invoice. `unitCents` is the per-unit price in integer cents;
// the line total is `quantity * unitCents`.
export interface InvoiceLineItem {
  description: string;
  quantity: number;
  unitCents: number;
}

// A bill tied to a client, + optional project (owner-only — money + PII).
// `amountCents` is the integer-cents total; `lineItems` is the JSON-encoded
// breakdown (parse with `parseLineItems`).
export interface InvoiceRow {
  id: string;
  number: string;
  clientId: string;
  clientName: string;
  projectId?: string | null;
  projectTitle?: string | null;
  lineItems?: string | null; // JSON array of InvoiceLineItem
  amountCents: number;
  status: string; // "draft" | "sent" | "paid" | "overdue"
  issuedAt?: string | null;
  dueAt?: string | null;
  notes?: string | null;
  createdAt: string;
}

// Owner-gated queries that carry private rows use the same discriminated shape
// as inquiries: `{ authorized: false }` for a non-owner, otherwise the data.
export type OwnerClientsResult =
  | { authorized: true; clients: ClientRow[] }
  | { authorized: false };

export type OwnerInvoicesResult =
  | { authorized: true; invoices: InvoiceRow[] }
  | { authorized: false };

/* ------------------------------ helpers ------------------------------ */

// Split a comma-separated tag string into trimmed, non-empty tags.
export function parseTags(tags?: string | null): string[] {
  return (tags ?? "")
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);
}

// Format integer cents as a currency string ("$1,500.00"). Whole-dollar amounts
// drop the cents ("$1,500").
export function money(cents: number): string {
  const dollars = (cents || 0) / 100;
  const whole = Number.isInteger(dollars);
  return dollars.toLocaleString("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: whole ? 0 : 2,
    maximumFractionDigits: 2,
  });
}

// Parse the JSON line-item blob off an invoice into a clean, typed array.
// Tolerant: bad JSON or missing fields yield an empty list / zeroed fields
// rather than throwing, so a malformed row never crashes the dashboard or PDF.
export function parseLineItems(raw?: string | null): InvoiceLineItem[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.map((it) => ({
      description: String(it?.description ?? ""),
      quantity: Number(it?.quantity) || 0,
      unitCents: Math.trunc(Number(it?.unitCents) || 0),
    }));
  } catch {
    return [];
  }
}

// Total of a line-item list, in integer cents.
export function lineItemsTotal(items: InvoiceLineItem[]): number {
  return items.reduce((sum, it) => sum + Math.round(it.quantity * it.unitCents), 0);
}

// Slugify a project title for its /work/[slug] URL.
export function slugify(s: string): string {
  return s
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60);
}

// A normalized project shape the public pages render — tags already split, the
// storage flags (selected/published/order) dropped. Built from a DB row
// (`viewFromRow`) or, before the portfolio is seeded, from a config case study,
// so the marketing pages render identically either way.
export interface ProjectView {
  slug: string;
  title: string;
  client: string;
  summary: string;
  year?: string | null;
  tags: string[];
  challenge?: string | null;
  approach?: string | null;
  outcome?: string | null;
  liveUrl?: string | null;
}

export function viewFromRow(r: ProjectRow): ProjectView {
  return {
    slug: r.slug,
    title: r.title,
    client: r.client,
    summary: r.summary,
    year: r.year,
    tags: parseTags(r.tags),
    challenge: r.challenge,
    approach: r.approach,
    outcome: r.outcome,
    liveUrl: r.liveUrl,
  };
}
