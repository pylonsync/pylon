import React from "react";
import { initials } from "@/lib/format";
import { cn } from "@/lib/utils";

// Messages-style monogram: a soft gradient disc with initials.
const TONES = [
  "from-[#a1a1aa] to-[#71717a]",
  "from-[#93c5fd] to-[#3b82f6]",
  "from-[#86efac] to-[#16a34a]",
  "from-[#fcd34d] to-[#d97706]",
  "from-[#f9a8d4] to-[#db2777]",
  "from-[#c4b5fd] to-[#7c3aed]",
];

function tone(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return TONES[h % TONES.length];
}

export function Avatar({
  name,
  handle,
  size = 40,
  className,
}: {
  name: string | null | undefined;
  handle: string;
  size?: number;
  className?: string;
}) {
  const known = Boolean(name && name.trim());
  return (
    <span
      aria-hidden
      className={cn(
        "inline-flex shrink-0 select-none items-center justify-center rounded-full bg-gradient-to-b font-semibold text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.25)]",
        known ? tone(handle) : TONES[0],
        className,
      )}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.38) }}
    >
      {initials(name, handle)}
    </span>
  );
}
