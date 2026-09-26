import React from "react";
import { cn } from "@/lib/utils";

/**
 * The Frontdesk mark: a speech bubble with two lines of text, on the accent
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
      <svg viewBox="0 0 16 16" className="size-[64%]" fill="none">
        <path
          d="M3.5 2.5h9a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2H8l-3 2.5v-2.5H3.5a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2Z"
          fill="currentColor"
        />
        <rect x="4.5" y="5.2" width="7" height="1.4" rx="0.7" fill="var(--primary)" />
        <rect x="4.5" y="7.6" width="4.5" height="1.4" rx="0.7" fill="var(--primary)" />
      </svg>
    </span>
  );
}
