import React, { useState } from "react";
import { cn } from "@/lib/utils";
import { DealCard } from "@/components/deal-card";
import { BOARD_STAGES, groupByStage, money, type Deal } from "@/lib/pipeline";

/**
 * The pipeline, as columns you can drag deals between.
 *
 * Presentational on purpose: it renders whatever deals it's handed and reports
 * a move through `onMove`. The container writes to `db`, the write syncs, and
 * the new rows arrive back here — which is also how a teammate's move appears,
 * so there is exactly one code path for "a deal changed stage".
 */
export function PipelineBoard({
  deals,
  companyName,
  ownerName,
  onMove,
  onOpen,
}: {
  deals: Deal[];
  companyName: (id: string | null | undefined) => string | null;
  ownerName: (id: string | null | undefined) => string | null;
  onMove: (dealId: string, stage: string) => void;
  onOpen: (dealId: string) => void;
}) {
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const columns = groupByStage(deals, BOARD_STAGES);

  return (
    <div className="flex h-full gap-3 overflow-x-auto p-4">
      {columns.map((column) => {
        const isTarget = over === column.stage.id;
        return (
          <section
            key={column.stage.id}
            onDragOver={(event) => {
              // Preventing default is what marks this a valid drop target.
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (over !== column.stage.id) setOver(column.stage.id);
            }}
            onDragLeave={(event) => {
              // Ignore bubbling from children, or the highlight flickers.
              if (event.currentTarget.contains(event.relatedTarget as Node)) return;
              setOver((current) => (current === column.stage.id ? null : current));
            }}
            onDrop={(event) => {
              event.preventDefault();
              const id = event.dataTransfer.getData("text/plain") || dragging;
              setOver(null);
              setDragging(null);
              if (!id) return;
              const deal = deals.find((d) => d.id === id);
              // A drop back on the same column is a no-op, not a write.
              if (!deal || deal.stage === column.stage.id) return;
              onMove(id, column.stage.id);
            }}
            className={cn(
              "flex w-[268px] shrink-0 flex-col rounded-xl border transition-colors",
              isTarget
                ? "border-ring/60 bg-surface-2/60"
                : "border-border bg-surface-1/50",
            )}
            aria-label={column.stage.label}
          >
            <header className="flex items-center gap-2 px-3 py-2.5">
              <h2 className="text-[12px] font-medium">{column.stage.label}</h2>
              <span className="tabular text-[11px] text-muted-foreground">
                {column.deals.length}
              </span>
              <span className="tabular ml-auto text-[11px] text-muted-foreground">
                {money(column.total)}
              </span>
            </header>

            <div className="flex min-h-24 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
              {column.deals.map((deal) => (
                <DealCard
                  key={deal.id}
                  deal={deal}
                  company={companyName(deal.companyId)}
                  owner={ownerName(deal.ownerId)}
                  dragging={dragging === deal.id}
                  onOpen={onOpen}
                  onDragStart={setDragging}
                  onDragEnd={() => {
                    setDragging(null);
                    setOver(null);
                  }}
                />
              ))}
              {column.deals.length === 0 ? (
                <p className="px-1 py-6 text-center text-[11px] text-muted-foreground">
                  {isTarget ? "Drop here" : "Nothing here"}
                </p>
              ) : null}
            </div>
          </section>
        );
      })}
    </div>
  );
}
