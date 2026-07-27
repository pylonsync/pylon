import React from "react";
import { cn } from "@/lib/utils";

/**
 * The bar at the top of every view: title on the left, actions on the right.
 * Fixed height so switching pages doesn't shift the content below it.
 */
export function PageHeader({
  title,
  count,
  children,
  className,
}: {
  title: string;
  /** Shown beside the title — the number of rows in view. */
  count?: number;
  children?: React.ReactNode;
  className?: string;
}) {
  return (
    <header
      className={cn(
        "flex h-12 shrink-0 items-center justify-between gap-3 border-b border-border px-4",
        className,
      )}
    >
      <div className="flex min-w-0 items-baseline gap-2">
        <h1 className="truncate text-[13px] font-semibold">{title}</h1>
        {typeof count === "number" ? (
          <span className="tabular text-[12px] text-muted-foreground">{count}</span>
        ) : null}
      </div>
      <div className="flex items-center gap-2">{children}</div>
    </header>
  );
}
