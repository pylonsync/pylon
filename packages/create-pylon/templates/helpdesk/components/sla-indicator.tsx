import React from "react";
import { cn } from "@/lib/utils";
import { duration } from "@/lib/format";
import { minutesToBreach, slaState, type Ticket } from "@/lib/tickets";

/**
 * How long is left to reply, or how long we're already late.
 *
 * Renders nothing once a ticket has been answered or closed — a queue where
 * every row carries a badge is a queue where none of them mean anything.
 */
export function SlaIndicator({
  ticket,
  now,
  className,
}: {
  ticket: Ticket;
  /** Injectable so the value is deterministic in a test. */
  now?: number;
  className?: string;
}) {
  const state = slaState(ticket, now);
  if (state === "none" || state === "met") return null;

  const minutes = minutesToBreach(ticket, now);
  if (minutes === null) return null;

  const breached = state === "breached";
  return (
    <span
      title={breached ? "First response overdue" : "First response due"}
      className={cn(
        "tabular inline-flex items-center gap-1 text-[11px] whitespace-nowrap",
        breached ? "text-destructive" : "text-muted-foreground",
        className,
      )}
    >
      <span
        className={cn(
          "size-1.5 rounded-full",
          breached ? "bg-destructive" : "bg-muted-foreground/60",
        )}
      />
      {breached ? `${duration(minutes)} over` : `${duration(minutes)} left`}
    </span>
  );
}
