import React from "react";
import { BOARD_STAGES } from "@/lib/pipeline";

/**
 * The board's shape while the replica hydrates.
 *
 * The shell is server-rendered but the deals arrive over sync, so there's a
 * beat before the first row lands. Showing the real column structure with
 * placeholder cards keeps that beat from reading as "you have no deals" — and
 * because the columns are already in place, nothing jumps when the data
 * arrives.
 */
export function BoardSkeleton() {
  return (
    <div className="flex h-full gap-3 overflow-hidden p-4" aria-hidden="true">
      {BOARD_STAGES.map((stage, column) => (
        <section
          key={stage.id}
          className="flex w-[268px] shrink-0 flex-col rounded-xl border border-border bg-surface-1/50"
        >
          <header className="flex items-center gap-2 px-3 py-2.5">
            <span className="text-[12px] font-medium text-muted-foreground">
              {stage.label}
            </span>
          </header>
          <div className="flex flex-col gap-2 px-2 pb-2">
            {/* Fewer cards further down the funnel, so the placeholder reads
                like a pipeline rather than a grid. */}
            {Array.from({ length: Math.max(1, 3 - column) }).map((_, card) => (
              <div
                key={card}
                className="animate-pulse rounded-lg border border-border bg-card p-2.5"
              >
                <div className="h-3 w-3/4 rounded bg-muted" />
                <div className="mt-2 h-2.5 w-1/2 rounded bg-muted" />
                <div className="mt-3 h-4 w-4 rounded-full bg-muted" />
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
