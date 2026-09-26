import React from "react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { closeDue, money, type Deal } from "@/lib/pipeline";

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
  // Only surface a date when it's actionable, and never on a closed deal.
  const due = closeDue(deal);

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
        "group cursor-pointer rounded-lg border border-border/80 bg-card p-3 transition-[border-color,box-shadow,opacity]",
        "shadow-[0_1px_2px_rgb(0_0_0/0.04)] hover:border-foreground/15 hover:shadow-[0_2px_6px_rgb(0_0_0/0.06)]",
        "focus-visible:border-ring focus-visible:outline-none",
        dragging && "opacity-40",
      )}
    >
      <p className="line-clamp-2 text-[13px] font-medium leading-snug">{deal.title}</p>
      {company ? (
        <p className="mt-0.5 truncate text-[12px] text-muted-foreground">{company}</p>
      ) : null}

      <div className="mt-3 flex items-center gap-2">
        <span className="tabular text-[13px] font-semibold tracking-[-0.01em]">
          {money(deal.value)}
        </span>
        {due ? (
          <span
            className={cn(
              "tabular rounded px-1.5 py-px text-[11px] font-medium",
              due.tone === "overdue"
                ? "bg-destructive/10 text-destructive"
                : "bg-foreground/[0.05] text-muted-foreground",
            )}
          >
            {due.label}
          </span>
        ) : null}
        {owner ? (
          <span className="ml-auto" title={`Owner: ${owner}`}>
            <Avatar name={owner} size="sm" />
          </span>
        ) : null}
      </div>
    </article>
  );
}
