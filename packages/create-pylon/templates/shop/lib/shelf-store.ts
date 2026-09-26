// Live shelf totals, shared between client islands. The product grid
// (app/(marketing)/shop-client.tsx) owns the live Product query and publishes
// the totals here; the hero's shelf count (components/shelf-count.tsx) reads
// them. One query and one guest session serve both islands.

import { useSyncExternalStore } from "react";

export type ShelfTotals = { units: number; low: number; soldOut: number } | null;

let totals: ShelfTotals = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function publishShelf(stocks: number[], lowAt: number) {
  const next = {
    units: stocks.reduce((s, n) => s + Math.max(0, n), 0),
    low: stocks.filter((n) => n > 0 && n <= lowAt).length,
    soldOut: stocks.filter((n) => n <= 0).length,
  };
  if (totals && totals.units === next.units && totals.low === next.low && totals.soldOut === next.soldOut) return;
  totals = next;
  listeners.forEach((l) => l());
}

export function useShelf(): ShelfTotals {
  return useSyncExternalStore(
    subscribe,
    () => totals,
    () => null,
  );
}
