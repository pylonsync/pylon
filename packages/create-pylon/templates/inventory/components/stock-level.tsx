import React from "react";
import { cn } from "@/lib/utils";
import { stockState } from "@/lib/stock";

const STYLE: Record<string, string> = {
  out: "text-destructive",
  low: "text-stage-proposal",
  ok: "text-foreground",
};

/**
 * On-hand, coloured by whether it needs attention.
 *
 * The number itself carries the colour rather than a separate badge: in a dense
 * table the quantity is what the eye goes to, and a chip beside it is a second
 * thing to read for the same information.
 */
export function StockLevel({
  quantity,
  reorderPoint,
  className,
}: {
  quantity: number;
  reorderPoint?: number | null;
  className?: string;
}) {
  const state = stockState(quantity, reorderPoint);
  return (
    <span
      className={cn("tabular font-medium", STYLE[state], className)}
      title={
        state === "out"
          ? "Out of stock"
          : state === "low"
            ? `At or below the reorder point of ${reorderPoint}`
            : undefined
      }
    >
      {quantity}
      {state === "out" ? (
        <span className="ml-1.5 text-[11px] font-normal">out</span>
      ) : state === "low" ? (
        <span className="ml-1.5 text-[11px] font-normal">low</span>
      ) : null}
    </span>
  );
}
