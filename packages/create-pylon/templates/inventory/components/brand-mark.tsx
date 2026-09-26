import React from "react";
import { cn } from "@/lib/utils";

/**
 * The Stockroom mark: three boxes stacked two over one, on the accent square.
 * app/icon.svg draws the same shape for the favicon.
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
        <rect x="4.6" y="1.5" width="6.8" height="5.8" rx="1.2" />
        <rect x="1" y="8.7" width="6.6" height="5.8" rx="1.2" />
        <rect x="8.4" y="8.7" width="6.6" height="5.8" rx="1.2" />
      </svg>
    </span>
  );
}
