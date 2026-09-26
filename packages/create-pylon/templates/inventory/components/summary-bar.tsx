import React from "react";
import { money, type Summary } from "@/lib/stock";

/**
 * What's on the shelves and what needs attention. All derived from the ledger —
 * see lib/stock.ts, where the arithmetic is unit-tested.
 */
export function SummaryBar({ summary: s }: { summary: Summary }) {
  return (
    <dl className="grid shrink-0 grid-cols-2 gap-px border-b border-border bg-border lg:grid-cols-4">
      <Metric label="Products" value={String(s.skuCount)} hint="active SKUs" />
      <Metric label="Units on hand" value={s.unitCount.toLocaleString("en-US")} hint="across all lines" />
      <Metric label="Stock value" value={money(s.valuationCents)} hint="at cost" />
      <Metric
        label="Needs reorder"
        value={String(s.lowCount + s.outCount)}
        hint={`${s.outCount} out of stock`}
        alarming={s.outCount > 0}
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
    <div className="min-w-0 bg-background px-4 py-2.5 md:px-5 md:py-3.5">
      <dt className="truncate text-[12px] text-muted-foreground">{label}</dt>
      <dd
        className={
          "tabular mt-1 text-[18px] font-semibold leading-none tracking-[-0.02em] md:text-[22px]" +
          (alarming ? " text-destructive" : "")
        }
      >
        {value}
      </dd>
      <p className="mt-1 truncate text-[11.5px] text-muted-foreground md:mt-1.5">{hint}</p>
    </div>
  );
}
