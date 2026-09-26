import React from "react";
import { cn } from "@/lib/utils";

/**
 * The Billfold mark: an invoice with a folded corner and two lines, on the
 * accent square. app/icon.svg draws the same shape for the favicon.
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
        <path d="M3 2.5A1.5 1.5 0 0 1 4.5 1h5l3.5 3.5v9A1.5 1.5 0 0 1 11.5 15h-7A1.5 1.5 0 0 1 3 13.5v-11Z" fill="currentColor" />
        <path d="M9.5 1v2.5a1 1 0 0 0 1 1H13" fill="var(--primary)" opacity="0.35" />
        <rect x="5.2" y="7.4" width="5.6" height="1.3" rx="0.65" fill="var(--primary)" />
        <rect x="5.2" y="10" width="3.6" height="1.3" rx="0.65" fill="var(--primary)" />
      </svg>
    </span>
  );
}
