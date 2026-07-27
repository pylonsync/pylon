import React from "react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { daysUntil, money, type Deal } from "@/lib/pipeline";

/**
 * One deal on the board. Draggable, but never the source of truth for where it
 * sits — the parent decides that from the synced row, so a teammate's move
 * lands here through the same path as your own.
 */
export function DealCard({
  deal,
  company,
  owner,
  onOpen,
  onDragStart,
  onDragEnd,
  dragging,
}: {
  deal: Deal;
  company?: string | null;
  owner?: string | null;
  onOpen: (id: string) => void;
  onDragStart: (id: string) => void;
  onDragEnd: () => void;
  dragging?: boolean;
}) {
  const days = daysUntil(deal.closeDate);
  // Only surface a date when it's actionable: overdue, or inside a week.
  const overdue = days !== null && days < 0;
  const soon = days !== null && days >= 0 && days <= 7;

  return (
    <article
      draggable
      onDragStart={(event) => {
        // setData is required for Firefox to start a drag at all.
        event.dataTransfer.setData("text/plain", deal.id);
        event.dataTransfer.effectAllowed = "move";
        onDragStart(deal.id);
      }}
      onDragEnd={onDragEnd}
      onClick={() => onOpen(deal.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen(deal.id);
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={`${deal.title}${company ? `, ${company}` : ""}`}
      className={cn(
        "group cursor-pointer rounded-lg border border-border bg-card p-2.5 transition-all",
        "hover:border-ring/40 focus-visible:border-ring",
        dragging && "opacity-40",
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <p className="line-clamp-2 flex-1 text-[13px] font-medium leading-snug">
          {deal.title}
        </p>
        <span className="tabular shrink-0 text-[12px] font-medium text-muted-foreground">
          {money(deal.value)}
        </span>
      </div>

      {company ? (
        <p className="mt-1 truncate text-[12px] text-muted-foreground">{company}</p>
      ) : null}

      <div className="mt-2.5 flex items-center justify-between gap-2">
        {owner ? <Avatar name={owner} size="sm" /> : <span />}
        {overdue || soon ? (
          <span
            className={cn(
              "tabular text-[11px]",
              overdue ? "text-destructive" : "text-muted-foreground",
            )}
          >
            {overdue
              ? `${Math.abs(days as number)}d overdue`
              : days === 0
                ? "Today"
                : `${days}d`}
          </span>
        ) : null}
      </div>
    </article>
  );
}
