import React from "react";
import { duration, money, type Portfolio } from "@/lib/work";

/**
 * The four numbers across active projects. Derived from the time ledger; see
 * `portfolio` in lib/work.ts, where the arithmetic is unit-tested.
 */
export function SummaryBar({ portfolio: p }: { portfolio: Portfolio }) {
  return (
    <dl className="grid shrink-0 grid-cols-2 gap-px border-b border-border bg-border lg:grid-cols-4">
      <Metric label="Active projects" value={String(p.activeCount)} hint="in delivery now" />
      <Metric label="Time logged" value={duration(p.loggedMinutes)} hint="on active projects" />
      <Metric label="Billable" value={money(p.billableCents)} hint="at each project's rate" />
      <Metric
        label="Over budget"
        value={String(p.overBudgetCount)}
        hint={p.overBudgetCount === 1 ? "project past its hours" : "projects past their hours"}
        alarming={p.overBudgetCount > 0}
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
