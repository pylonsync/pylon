import React from "react";
import { money, summarize, type Movement, type Product } from "@/lib/stock";

/**
 * What\'s on the shelves and what needs attention. All derived from the ledger —
 * see lib/stock.ts, where the arithmetic is unit-tested.
 */
export function SummaryBar({
  products,
  movements,
}: {
  products: Product[];
  movements: Movement[];
}) {
  const s = summarize(products, movements);
  return (
    <dl className="grid grid-cols-2 gap-px border-b border-border bg-border sm:grid-cols-4">
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
