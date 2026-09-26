import React from "react";
import { cn } from "@/lib/utils";
import { statusById } from "@/lib/billing";

// "overdue" isn't a stored status — it's derived in lib/billing.ts — but it's
// the one the eye needs to find first, so it gets the only alarming colour.
const STYLE: Record<string, string> = {
  draft: "bg-foreground/[0.06] text-muted-foreground",
  sent: "bg-stage-qualified/10 text-stage-qualified",
  overdue: "bg-destructive/10 text-destructive",
  paid: "bg-stage-won/10 text-stage-won",
  void: "bg-foreground/[0.06] text-muted-foreground line-through",
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
        "inline-flex h-5 items-center rounded px-1.5 text-[11.5px] font-medium whitespace-nowrap",
        STYLE[status] ?? "bg-foreground/[0.06] text-muted-foreground",
        className,
      )}
    >
      {LABEL[status] ?? statusById(status)?.label ?? status}
    </span>
  );
}
