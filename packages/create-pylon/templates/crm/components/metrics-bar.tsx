import React from "react";
import { metrics, money, percent, type Deal } from "@/lib/pipeline";

/**
 * The four numbers a sales lead actually looks at. Derived, never stored — see
 * lib/pipeline.ts, where the arithmetic is unit-tested.
 */
export function MetricsBar({ deals }: { deals: Deal[] }) {
  const m = metrics(deals);
  return (
    <dl className="grid grid-cols-2 gap-px border-b border-border bg-border sm:grid-cols-4">
      <Metric
        label="Open pipeline"
        value={money(m.open)}
        hint={`${m.openCount} deal${m.openCount === 1 ? "" : "s"}`}
      />
      <Metric
        label="Weighted"
        value={money(m.weighted)}
        hint="by stage probability"
      />
      <Metric label="Won" value={money(m.won)} hint="closed" />
      <Metric label="Win rate" value={percent(m.winRate)} hint="of closed deals" />
    </dl>
  );
}

function Metric({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <div className="bg-background px-4 py-3">
      <dt className="text-[11px] text-muted-foreground">{label}</dt>
      <dd className="tabular mt-0.5 text-[18px] font-semibold tracking-tight">
        {value}
      </dd>
      <p className="mt-0.5 text-[11px] text-muted-foreground">{hint}</p>
    </div>
  );
}
