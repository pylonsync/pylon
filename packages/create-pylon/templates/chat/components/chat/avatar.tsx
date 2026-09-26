import React from "react";
import { cn } from "@/lib/utils";
import { initials } from "@/lib/chat";

interface AvatarProps {
  name: string;
  hue: number;
  size?: "xs" | "sm" | "md";
  online?: boolean;
  className?: string;
}

const SIZES = {
  xs: "size-5 text-[9px]",
  sm: "size-7 text-[11px]",
  md: "size-9 text-[13px]",
} as const;

/** Initials on a circle tinted by the member's hue. */
export function Avatar({ name, hue, size = "md", online, className }: AvatarProps) {
  return (
    <span className={cn("relative inline-flex shrink-0", className)}>
      <span
        aria-hidden
        className={cn(
          "inline-flex items-center justify-center rounded-full font-semibold tracking-tight select-none",
          SIZES[size],
        )}
        style={{
          backgroundColor: `oklch(0.9 0.05 ${hue})`,
          color: `oklch(0.38 0.1 ${hue})`,
          boxShadow: `inset 0 0 0 1px oklch(0.8 0.07 ${hue} / 0.6)`,
        }}
      >
        {initials(name)}
      </span>
      {online ? (
        <span
          aria-hidden
          className="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full bg-online ring-2 ring-[var(--avatar-ring,var(--background))]"
        />
      ) : null}
    </span>
  );
}
