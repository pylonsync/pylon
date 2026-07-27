import React from "react";
import { cn } from "@/lib/utils";
import { priorityById, statusById } from "@/lib/tickets";

const PRIORITY_DOT: Record<string, string> = {
  urgent: "bg-stage-lost",
  high: "bg-stage-proposal",
  normal: "bg-stage-qualified",
  low: "bg-stage-lead",
};

/** Priority as a dot plus a label — readable in a dense row without a chip. */
export function PriorityBadge({
  priority,
  className,
}: {
  priority: string;
  className?: string;
}) {
  const meta = priorityById(priority);
  return (
    <span className={cn("inline-flex items-center gap-1.5 whitespace-nowrap", className)}>
      <span
        className={cn(
          "size-1.5 shrink-0 rounded-full",
          PRIORITY_DOT[priority] ?? "bg-muted-foreground",
        )}
      />
      <span className="text-muted-foreground">{meta?.label ?? priority}</span>
    </span>
  );
}

const STATUS_STYLE: Record<string, string> = {
  open: "border-stage-qualified/40 text-stage-qualified",
  pending: "border-stage-proposal/40 text-stage-proposal",
  solved: "border-stage-won/40 text-stage-won",
  closed: "border-border text-muted-foreground",
};

/** Status as an outlined chip — it's the field agents filter on most. */
export function StatusBadge({
  status,
  className,
}: {
  status: string;
  className?: string;
}) {
  const meta = statusById(status);
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center rounded border px-1.5 text-[11px] font-medium whitespace-nowrap",
        STATUS_STYLE[status] ?? "border-border text-muted-foreground",
        className,
      )}
    >
      {meta?.label ?? status}
    </span>
  );
}
