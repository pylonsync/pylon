import React from "react";
import { cn } from "@/lib/utils";
import { budgetState, duration, money, billableCents } from "@/lib/work";

/**
 * Budget consumed, as a bar plus the figures.
 *
 * Deliberately separate from task progress: time spent measures effort, not
 * completion. A project can be 90% through its budget and 20% done, and a
 * status report that conflates them lies.
 */
export function BudgetBar({
  budgetMinutes,
  loggedMinutes,
  hourlyRateCents,
}: {
  budgetMinutes?: number | null;
  loggedMinutes: number;
  hourlyRateCents?: number | null;
}) {
  const budget = Number(budgetMinutes) || 0;
  const state = budgetState(budget, loggedMinutes);
  const pct = budget > 0 ? Math.min(100, (loggedMinutes / budget) * 100) : 0;

  return (
    <div className="space-y-1.5">
      <div className="flex items-baseline justify-between gap-3 text-[12px]">
        <span className="text-muted-foreground">
          {state === "none"
            ? "No budget set"
            : `${duration(loggedMinutes)} of ${duration(budget)}`}
        </span>
        {hourlyRateCents ? (
          <span className="tabular text-muted-foreground">
            {money(billableCents(loggedMinutes, hourlyRateCents))} billable
          </span>
        ) : null}
      </div>
      {state !== "none" ? (
        <div className="h-1.5 overflow-hidden rounded-full bg-muted">
          <div
            className={cn(
              "h-full rounded-full transition-all",
              state === "over"
                ? "bg-destructive"
                : state === "near"
                  ? "bg-stage-proposal"
                  : "bg-stage-won",
            )}
            style={{ width: `${Math.max(2, pct)}%` }}
          />
        </div>
      ) : null}
      {state === "over" ? (
        <p className="text-[11px] text-destructive">
          Over budget by {duration(loggedMinutes - budget)}
        </p>
      ) : null}
    </div>
  );
}
