import React from "react";
import { cn } from "@/lib/utils";

/** A keyboard hint. Present in the UI because the shortcuts are real. */
export function Kbd({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center rounded border border-border bg-surface-2 px-1.5",
        "font-sans text-[10px] font-medium text-muted-foreground",
        className,
      )}
    >
      {children}
    </kbd>
  );
}
