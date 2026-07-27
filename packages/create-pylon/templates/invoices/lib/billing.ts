// Invoice arithmetic and status.
//
// MONEY IS INTEGER CENTS everywhere in this app. Floating-point dollars are how
// an invoice ends up off by a penny after tax, and a penny on an invoice is a
// support ticket. Values are converted at the edges — `parseAmount` on input,
// `money` on output — and never in between.
//
// Pure: no React, no `db`. The totals a customer is billed from are the last
// thing that should need a running server to verify.

export interface Status {
  id: string;
  label: string;
}

export const STATUSES: Status[] = [
  { id: "draft", label: "Draft" },
  { id: "sent", label: "Sent" },
  { id: "paid", label: "Paid" },
  { id: "void", label: "Void" },
];

export interface LineItem {
  id: string;
  invoiceId: string;
  description: string;
  /** Thousandths, so 1.5 hours is 1500 — no float quantities either. */
  quantityMilli: number;
  unitPriceCents: number;
}

export interface Payment {
  id: string;
  invoiceId: string;
  amountCents: number;
  paidAt?: string | null;
  method?: string | null;
}

export interface Invoice {
  id: string;
  number: string;
  clientId?: string | null;
  status: string;
  /** Basis points: 875 = 8.75%. Integer, for the same reason as cents. */
  taxRateBps?: number | null;
  issueDate?: string | null;
  dueDate?: string | null;
  notes?: string | null;
  createdAt?: string | null;
}

export function statusById(id: string): Status | undefined {
  return STATUSES.find((s) => s.id === id);
}

export function isValidStatus(id: string): boolean {
  return STATUSES.some((s) => s.id === id);
}

export interface Totals {
  subtotalCents: number;
  taxCents: number;
  totalCents: number;
  paidCents: number;
  balanceCents: number;
}

/**
 * What an invoice is worth, what's been paid, and what's left.
 *
 * Tax is computed on the rounded subtotal rather than per line: that's what a
 * customer can reproduce from the printed invoice, and per-line rounding drifts
 * from the visible total.
 */
export function totals(
  invoice: Invoice,
  items: LineItem[],
  payments: Payment[],
): Totals {
  const lines = items.filter((item) => item.invoiceId === invoice.id);
  const subtotalCents = lines.reduce((sum, item) => sum + lineTotalCents(item), 0);

  const bps = Math.max(0, Number(invoice.taxRateBps) || 0);
  const taxCents = Math.round((subtotalCents * bps) / 10_000);
  const totalCents = subtotalCents + taxCents;

  const paidCents = payments
    .filter((payment) => payment.invoiceId === invoice.id)
    .reduce((sum, payment) => sum + (Number(payment.amountCents) || 0), 0);

  return {
    subtotalCents,
    taxCents,
    totalCents,
    paidCents,
    // Never negative: an overpayment is a credit to handle deliberately, not a
    // negative balance that quietly offsets the next invoice.
    balanceCents: Math.max(0, totalCents - paidCents),
  };
}

/** One line, rounded once. quantityMilli is thousandths of a unit. */
export function lineTotalCents(item: {
  quantityMilli: number;
  unitPriceCents: number;
}): number {
  const qty = Number(item.quantityMilli) || 0;
  const unit = Number(item.unitPriceCents) || 0;
  return Math.round((qty * unit) / 1000);
}

/**
 * Overdue is DERIVED, not a stored status — an invoice becomes overdue by the
 * clock passing, and no job runs at midnight to flip a column. A sent invoice
 * with a due date in the past and a balance outstanding is overdue; anything
 * paid, void, or still a draft is not.
 */
export function isOverdue(
  invoice: Invoice,
  balanceCents: number,
  now: number = Date.now(),
): boolean {
  if (invoice.status !== "sent") return false;
  if (balanceCents <= 0) return false;
  const due = Date.parse(invoice.dueDate ?? "");
  if (!Number.isFinite(due)) return false;
  return now > endOfDay(due);
}

/** What to show in the status column, including the derived Overdue. */
export function displayStatus(
  invoice: Invoice,
  balanceCents: number,
  now: number = Date.now(),
): string {
  if (isOverdue(invoice, balanceCents, now)) return "overdue";
  // A sent invoice that's been fully paid reads as paid even before someone
  // remembers to change the field.
  if (invoice.status === "sent" && balanceCents <= 0) return "paid";
  return invoice.status;
}

export function daysOverdue(
  invoice: Invoice,
  now: number = Date.now(),
): number | null {
  const due = Date.parse(invoice.dueDate ?? "");
  if (!Number.isFinite(due)) return null;
  return Math.floor((startOfDay(now) - startOfDay(due)) / 86_400_000);
}

export interface Summary {
  outstandingCents: number;
  overdueCents: number;
  paidCents: number;
  draftCount: number;
  overdueCount: number;
}

/** The numbers on the list header: what's owed, what's late, what's landed. */
export function summarize(
  invoices: Invoice[],
  items: LineItem[],
  payments: Payment[],
  now: number = Date.now(),
): Summary {
  const summary: Summary = {
    outstandingCents: 0,
    overdueCents: 0,
    paidCents: 0,
    draftCount: 0,
    overdueCount: 0,
  };

  for (const invoice of invoices) {
    const t = totals(invoice, items, payments);
    summary.paidCents += t.paidCents;
    if (invoice.status === "draft") summary.draftCount += 1;
    if (invoice.status === "sent") {
      summary.outstandingCents += t.balanceCents;
      if (isOverdue(invoice, t.balanceCents, now)) {
        summary.overdueCents += t.balanceCents;
        summary.overdueCount += 1;
      }
    }
  }
  return summary;
}

/** "$1,240.00" — invoices show exact amounts, never a compacted $1.2K. */
export function money(cents: number | null | undefined): string {
  const value = Number(cents) || 0;
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  const dollars = Math.floor(abs / 100).toLocaleString("en-US");
  const rest = String(abs % 100).padStart(2, "0");
  return `${sign}$${dollars}.${rest}`;
}

/** "1.5" from 1500 thousandths, without a trailing ".000". */
export function quantity(milli: number | null | undefined): string {
  const value = (Number(milli) || 0) / 1000;
  return String(Number(value.toFixed(3)));
}

/**
 * Parse a typed amount into cents. Accepts "1,240.50", "$1240.5", "1240".
 * Returns null for anything it can't read, so a bad keystroke can't silently
 * become $0.00 on a bill.
 */
export function parseAmount(input: string): number | null {
  const cleaned = input.replace(/[$,\s]/g, "");
  if (!cleaned || !/^-?\d*\.?\d*$/.test(cleaned) || cleaned === "." || cleaned === "-") {
    return null;
  }
  const value = Number(cleaned);
  if (!Number.isFinite(value)) return null;
  return Math.round(value * 100);
}

/** Parse a typed quantity into thousandths. */
export function parseQuantity(input: string): number | null {
  const cleaned = input.replace(/[,\s]/g, "");
  if (!cleaned || !/^\d*\.?\d*$/.test(cleaned) || cleaned === ".") return null;
  const value = Number(cleaned);
  if (!Number.isFinite(value)) return null;
  return Math.round(value * 1000);
}

/**
 * The next invoice number, continuing whatever series is already there.
 *
 * Derived from the existing numbers rather than a stored counter: a counter in
 * a synced replica is a race, and gaps in an invoice series get asked about by
 * accountants.
 */
export function nextNumber(existing: string[], year: number): string {
  const prefix = `INV-${year}-`;
  let highest = 0;
  for (const number of existing) {
    if (!number.startsWith(prefix)) continue;
    const suffix = Number(number.slice(prefix.length));
    if (Number.isFinite(suffix) && suffix > highest) highest = suffix;
  }
  return `${prefix}${String(highest + 1).padStart(4, "0")}`;
}

function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function endOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(23, 59, 59, 999);
  return date.getTime();
}
