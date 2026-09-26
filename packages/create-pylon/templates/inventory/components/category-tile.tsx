import React from "react";
import { Coffee, Leaf, Milk, Package, Shirt, Wrench } from "lucide-react";
import { cn } from "@/lib/utils";
import { accentIndex } from "@/lib/format";

// Icon and tint per seeded category. An unknown category gets a box and a
// tint picked from its name, so a new category works without touching this
// file.
const KNOWN: Record<string, { icon: React.ReactNode; tint: string }> = {
  Coffee: { icon: <Coffee />, tint: "bg-amber-600/12 text-amber-800" },
  Tea: { icon: <Leaf />, tint: "bg-emerald-600/12 text-emerald-700" },
  "Dairy & Alternatives": { icon: <Milk />, tint: "bg-sky-500/12 text-sky-700" },
  Equipment: { icon: <Wrench />, tint: "bg-slate-500/12 text-slate-700" },
  Merch: { icon: <Shirt />, tint: "bg-violet-500/12 text-violet-700" },
  Packaging: { icon: <Package />, tint: "bg-orange-500/12 text-orange-700" },
};

const TINTS = [
  "bg-amber-500/12 text-amber-700",
  "bg-emerald-500/12 text-emerald-700",
  "bg-sky-500/12 text-sky-700",
  "bg-rose-500/12 text-rose-700",
  "bg-violet-500/12 text-violet-700",
  "bg-stone-500/12 text-stone-700",
];

/** A small square that marks a product's category in dense lists. */
export function CategoryTile({
  category,
  className,
}: {
  category: string | null | undefined;
  className?: string;
}) {
  const name = category ?? "";
  return (
    <span
      aria-hidden="true"
      title={name || undefined}
      className={cn(
        "inline-flex size-7 shrink-0 items-center justify-center rounded-md [&_svg]:size-3.5",
        KNOWN[name]?.tint ?? TINTS[accentIndex(name || "?", TINTS.length)],
        className,
      )}
    >
      {KNOWN[name]?.icon ?? <Package />}
    </span>
  );
}
