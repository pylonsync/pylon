import React from "react";
import { money, summarize, type Invoice, type LineItem, type Payment } from "@/lib/billing";

/**
 * The four numbers a small business checks daily. All derived from the line
 * items and payments — see lib/billing.ts, where the arithmetic is unit-tested.
 */
export function SummaryBar({
  invoices,
  items,
  payments,
  now,
}: {
  invoices: Invoice[];
  items: LineItem[];
  payments: Payment[];
  /** Injectable so a test gets a deterministic "overdue". */
  now?: number;
}) {
  const s = summarize(invoices, items, payments, now);
  return (
    <dl className="grid grid-cols-2 gap-px border-b border-border bg-border sm:grid-cols-4">
      <Metric label="Outstanding" value={money(s.outstandingCents)} hint="sent, unpaid" />
      <Metric
        label="Overdue"
        value={money(s.overdueCents)}
        hint={`${s.overdueCount} invoice${s.overdueCount === 1 ? "" : "s"}`}
        alarming={s.overdueCents > 0}
      />
      <Metric label="Collected" value={money(s.paidCents)} hint="all time" />
      <Metric
        label="Drafts"
        value={String(s.draftCount)}
        hint="not yet sent"
      />
    </dl>
  );
}

function Metric({
  label,
  value,
  hint,
  alarming,
}: {
  label: string;
  value: string;
  hint: string;
  alarming?: boolean;
}) {
  return (
    <div className="bg-background px-4 py-3">
      <dt className="text-[11px] text-muted-foreground">{label}</dt>
      <dd
        className={
          "tabular mt-0.5 text-[18px] font-semibold tracking-tight" +
          (alarming ? " text-destructive" : "")
        }
      >
        {value}
      </dd>
      <p className="mt-0.5 text-[11px] text-muted-foreground">{hint}</p>
    </div>
  );
}
