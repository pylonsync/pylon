"use client";

import React from "react";
import { useShelf } from "@/lib/shelf-store";

// The live stock line in the hero. The product grid publishes shelf totals to
// lib/shelf-store.ts from its live Product query, so this count drops for
// every open page when someone checks out. Renders a fixed-width placeholder
// until the first totals arrive, so the hero does not shift.
export function ShelfCount() {
  const shelf = useShelf();
  return (
    <p className="flex items-center gap-3 text-[13.5px] text-ink/75" aria-live="polite">
      <span className="relative flex size-2">
        <span className="ping-soft absolute inset-0 rounded-full bg-[#5d7a4a]" />
        <span className="relative size-2 rounded-full bg-[#5d7a4a]" />
      </span>
      {shelf ? (
        <span>
          <span key={shelf.units} className="stock-tick rounded px-0.5 font-medium tabular-nums text-ink">
            {shelf.units}
          </span>{" "}
          pieces on the studio shelf
          {shelf.low > 0 ? (
            <>
              <span className="mx-2 text-ink/30">/</span>
              <span className="tabular-nums">{shelf.low}</span> running low
            </>
          ) : null}
        </span>
      ) : (
        <span className="inline-block h-4 w-56 rounded bg-ink/[0.06]" />
      )}
    </p>
  );
}
