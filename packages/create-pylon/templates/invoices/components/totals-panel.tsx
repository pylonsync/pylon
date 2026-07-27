import React from "react";
import { cn } from "@/lib/utils";
import { money, type Totals } from "@/lib/billing";

/**
 * Subtotal → tax → total → paid → balance, in that order, because that's the
 * order a customer reads it in and the order they'll query if it doesn't add up.
 * Tax is applied to the rounded subtotal so the printed figures reconcile.
 */
export function TotalsPanel({
  totals,
  taxRateBps,
}: {
  totals: Totals;
  taxRateBps?: number | null;
}) {
  const rate = (Number(taxRateBps) || 0) / 100;
  return (
    <dl className="ml-auto w-full max-w-xs space-y-1.5 text-[13px]">
      <Row label="Subtotal" value={money(totals.subtotalCents)} />
      {totals.taxCents > 0 || rate > 0 ? (
        <Row label={`Tax (${Number(rate.toFixed(2))}%)`} value={money(totals.taxCents)} />
      ) : null}
      <Row label="Total" value={money(totals.totalCents)} strong />
      {totals.paidCents > 0 ? (
        <Row label="Paid" value={`−${money(totals.paidCents)}`} muted />
      ) : null}
      <div className="border-t border-border pt-1.5">
        <Row
          label="Balance due"
          value={money(totals.balanceCents)}
          strong
          alarming={totals.balanceCents > 0}
        />
      </div>
    </dl>
  );
}

function Row({
  label,
  value,
  strong,
  muted,
  alarming,
}: {
  label: string;
  value: string;
  strong?: boolean;
  muted?: boolean;
  alarming?: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-4">
      <dt className={cn("text-muted-foreground", strong && "text-foreground")}>
        {label}
      </dt>
      <dd
        className={cn(
          "tabular",
          strong && "font-semibold",
          muted && "text-muted-foreground",
          alarming && "text-destructive",
        )}
      >
        {value}
      </dd>
    </div>
  );
}
