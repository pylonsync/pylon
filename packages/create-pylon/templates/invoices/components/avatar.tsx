import React from "react";
import { cn } from "@/lib/utils";
import { accentIndex, initials } from "@/lib/format";

// Deterministic accent per record — the same company keeps its colour across
// sessions and machines without storing one. Hues are muted so a dense list
// doesn't turn into confetti.
const ACCENTS = [
  "bg-indigo-500/15 text-indigo-500 dark:text-indigo-300",
  "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300",
  "bg-amber-500/15 text-amber-600 dark:text-amber-300",
  "bg-rose-500/15 text-rose-500 dark:text-rose-300",
  "bg-sky-500/15 text-sky-600 dark:text-sky-300",
  "bg-violet-500/15 text-violet-500 dark:text-violet-300",
];

export function Avatar({
  name,
  size = "md",
  className,
}: {
  name: string | null | undefined;
  size?: "sm" | "md";
  className?: string;
}) {
  const label = initials(name);
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex shrink-0 select-none items-center justify-center rounded-full font-medium",
        size === "sm" ? "size-5 text-[9px]" : "size-6 text-[10px]",
        ACCENTS[accentIndex(name ?? "?", ACCENTS.length)],
        className,
      )}
    >
      {label}
    </span>
  );
}
