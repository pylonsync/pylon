import React from "react";
import { cn } from "@/lib/utils";

const TONE: Record<string, string> = {
  primary: "bg-primary",
  neutral: "bg-foreground/45",
  warn: "bg-stage-proposal",
  danger: "bg-destructive",
};

/**
 * A thin horizontal bar with its figure beside it, for progress and budget
 * columns. `ratio` is clamped to 0–1 for the bar; the label carries the
 * real numbers.
 */
export function Meter({
  ratio,
  label,
  tone = "primary",
  title,
  className,
}: {
  ratio: number;
  label: string;
  tone?: keyof typeof TONE;
  title?: string;
  className?: string;
}) {
  const pct = Math.max(0, Math.min(1, Number.isFinite(ratio) ? ratio : 0)) * 100;
  return (
    <span className={cn("flex min-w-0 items-center gap-2.5", className)} title={title}>
      <span
        aria-hidden="true"
        className="h-1.5 min-w-10 flex-1 overflow-hidden rounded-full bg-foreground/[0.08]"
      >
        <span className={cn("block h-full rounded-full", TONE[tone])} style={{ width: `${pct}%` }} />
      </span>
      <span
        className={cn(
          "tabular shrink-0 text-[12.5px]",
          tone === "danger" ? "font-medium text-destructive" : "text-muted-foreground",
        )}
      >
        {label}
      </span>
    </span>
  );
}
