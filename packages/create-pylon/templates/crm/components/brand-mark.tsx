import React from "react";
import { cn } from "@/lib/utils";

/**
 * The Dealbook mark: three stacked bars that narrow like a sales funnel, on
 * the accent square. app/icon.svg draws the same shape for the favicon.
 */
export function BrandMark({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex size-6 shrink-0 items-center justify-center rounded-[7px] bg-primary text-primary-foreground",
        "shadow-[inset_0_1px_0_rgb(255_255_255/0.18),0_1px_2px_rgb(0_0_0/0.12)]",
        className,
      )}
    >
      <svg viewBox="0 0 16 16" className="size-[62%]" fill="currentColor">
        <rect x="1.5" y="2.5" width="13" height="2.6" rx="1.3" />
        <rect x="3.5" y="6.7" width="9" height="2.6" rx="1.3" />
        <rect x="5.5" y="10.9" width="5" height="2.6" rx="1.3" />
      </svg>
    </span>
  );
}
