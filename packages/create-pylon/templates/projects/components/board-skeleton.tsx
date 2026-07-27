import React from "react";
import { TASK_STATUSES } from "@/lib/work";

/**
 * The board\'s shape while the replica hydrates. Showing the real columns keeps
 * the beat before the first row lands from reading as "this project has no
 * tasks", and nothing jumps when the data arrives.
 */
export function BoardSkeleton() {
  return (
    <div className="flex h-full gap-3 overflow-hidden p-4" aria-hidden="true">
      {TASK_STATUSES.map((status, column) => (
        <section
          key={status.id}
          className="flex w-[268px] shrink-0 flex-col rounded-xl border border-border bg-surface-1/50"
        >
          <header className="flex items-center gap-2 px-3 py-2.5">
            <span className="text-[12px] font-medium text-muted-foreground">
              {status.label}
            </span>
          </header>
          <div className="flex flex-col gap-2 px-2 pb-2">
            {Array.from({ length: Math.max(1, 3 - column) }).map((_, card) => (
              <div
                key={card}
                className="animate-pulse rounded-lg border border-border bg-card p-2.5"
              >
                <div className="h-3 w-3/4 rounded bg-muted" />
                <div className="mt-3 h-4 w-4 rounded-full bg-muted" />
              </div>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
