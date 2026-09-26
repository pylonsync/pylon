import React from "react";
import { metrics, money, percent, type Deal } from "@/lib/pipeline";

/**
 * The four numbers a sales lead actually looks at. Derived, never stored — see
 * lib/pipeline.ts, where the arithmetic is unit-tested.
 */
export function MetricsBar({ deals }: { deals: Deal[] }) {
  const m = metrics(deals);
  return (
    <dl className="grid shrink-0 grid-cols-2 gap-px border-b border-border bg-border lg:grid-cols-4">
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
      <Metric label="Won" value={money(m.won)} hint="closed won" />
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
    <div className="min-w-0 bg-background px-4 py-2.5 md:px-5 md:py-3.5">
      <dt className="truncate text-[12px] text-muted-foreground">{label}</dt>
      <dd className="tabular mt-1 text-[18px] font-semibold leading-none tracking-[-0.02em] md:text-[22px]">
        {value}
      </dd>
      <p className="mt-1 truncate text-[11.5px] text-muted-foreground md:mt-1.5">{hint}</p>
    </div>
  );
}
