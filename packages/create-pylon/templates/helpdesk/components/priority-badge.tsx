import React from "react";
import { cn } from "@/lib/utils";
import { priorityById, statusById } from "@/lib/tickets";

const PRIORITY_DOT: Record<string, string> = {
  urgent: "bg-stage-lost",
  high: "bg-stage-proposal",
  normal: "bg-stage-qualified",
  low: "bg-stage-lead",
};

const BARS: Record<string, number> = { high: 3, normal: 2, low: 1 };

/**
 * Priority as a 16px glyph for dense rows: a filled alert square for urgent,
 * and one to three signal bars for low, normal, and high.
 */
export function PriorityIcon({
  priority,
  title,
  className,
}: {
  priority: string;
  title?: string;
  className?: string;
}) {
  if (priority === "urgent") {
    return (
      <svg viewBox="0 0 16 16" role="img" aria-label={title ?? "Urgent"} className={cn("size-4 text-destructive", className)}>
        <rect x="1.5" y="1.5" width="13" height="13" rx="3" fill="currentColor" />
        <rect x="7.1" y="4" width="1.8" height="5.2" rx="0.9" fill="white" />
        <rect x="7.1" y="10.4" width="1.8" height="1.8" rx="0.9" fill="white" />
      </svg>
    );
  }
  const filled = BARS[priority] ?? 0;
  return (
    <svg viewBox="0 0 16 16" role="img" aria-label={title ?? priority} className={cn("size-4 text-foreground/75", className)}>
      {[0, 1, 2].map((bar) => (
        <rect
          key={bar}
          x={2 + bar * 4.5}
          y={10 - bar * 3.5}
          width="3"
          height={4 + bar * 3.5}
          rx="1"
          fill="currentColor"
          opacity={bar < filled ? 1 : 0.2}
        />
      ))}
    </svg>
  );
}

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
  open: "bg-stage-qualified/10 text-stage-qualified",
  pending: "bg-stage-proposal/12 text-[color-mix(in_oklab,var(--stage-proposal)_80%,black)]",
  solved: "bg-stage-won/10 text-stage-won",
  closed: "bg-foreground/[0.06] text-muted-foreground",
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
        "inline-flex h-5 items-center rounded px-1.5 text-[11.5px] font-medium whitespace-nowrap",
        STATUS_STYLE[status] ?? "bg-foreground/[0.06] text-muted-foreground",
        className,
      )}
    >
      {meta?.label ?? status}
    </span>
  );
}
