import React from "react";
import { cn } from "@/lib/utils";
import { statusById } from "@/lib/billing";

// "overdue" isn't a stored status — it's derived in lib/billing.ts — but it's
// the one the eye needs to find first, so it gets the only alarming colour.
const STYLE: Record<string, string> = {
  draft: "border-border text-muted-foreground",
  sent: "border-stage-qualified/40 text-stage-qualified",
  overdue: "border-destructive/50 text-destructive",
  paid: "border-stage-won/40 text-stage-won",
  void: "border-border text-muted-foreground line-through",
};

const LABEL: Record<string, string> = { overdue: "Overdue" };

export function StatusBadge({
  status,
  className,
}: {
  status: string;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center rounded border px-1.5 text-[11px] font-medium whitespace-nowrap",
        STYLE[status] ?? "border-border text-muted-foreground",
        className,
      )}
    >
      {LABEL[status] ?? statusById(status)?.label ?? status}
    </span>
  );
}
