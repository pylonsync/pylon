import React from "react";
import { cn } from "@/lib/utils";

/**
 * The Workbench mark: three board columns of falling height, on the accent
 * square. app/icon.svg draws the same shape for the favicon.
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
        <rect x="1.5" y="1.5" width="3.6" height="13" rx="1.2" />
        <rect x="6.2" y="1.5" width="3.6" height="9.5" rx="1.2" />
        <rect x="10.9" y="1.5" width="3.6" height="6" rx="1.2" />
      </svg>
    </span>
  );
}
