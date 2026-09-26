import React from "react";
import { cn } from "@/lib/utils";
import { stockState } from "@/lib/stock";

const TEXT: Record<string, string> = {
  out: "text-destructive",
  low: "text-[color-mix(in_oklab,var(--stage-proposal)_75%,black)]",
  ok: "text-foreground",
};

const BAR: Record<string, string> = {
  out: "bg-destructive",
  low: "bg-stage-proposal",
  ok: "bg-primary/70",
};

/**
 * On-hand as a number and a short bar, coloured by whether it needs
 * attention. The bar is full at three times the reorder point, so a line
 * sitting at its reorder point shows a third full.
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
  const point = Number(reorderPoint) || 0;
  const scale = point > 0 ? point * 3 : Math.max(quantity, 1);
  const fill = Math.max(0, Math.min(1, quantity / scale));
  return (
    <span
      className={cn("inline-flex items-center justify-end gap-3", className)}
      title={
        state === "out"
          ? "Out of stock"
          : state === "low"
            ? `At or below the reorder point of ${reorderPoint}`
            : undefined
      }
    >
      <span
        aria-hidden="true"
        className="hidden h-1.5 w-14 overflow-hidden rounded-full bg-foreground/[0.08] md:inline-block"
      >
        <span
          className={cn("block h-full rounded-full", BAR[state])}
          style={{ width: `${Math.max(fill * 100, state === "out" ? 0 : 6)}%` }}
        />
      </span>
      <span className={cn("tabular min-w-[3.25rem] text-right font-medium", TEXT[state])}>
        {quantity}
        {state !== "ok" ? (
          <span className="ml-1 text-[11px] font-normal">{state === "out" ? "out" : "low"}</span>
        ) : null}
      </span>
    </span>
  );
}
