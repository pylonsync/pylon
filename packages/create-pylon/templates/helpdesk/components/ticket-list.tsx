import React from "react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { PriorityIcon, StatusBadge } from "@/components/priority-badge";
import { SlaIndicator } from "@/components/sla-indicator";
import { relativeTime } from "@/lib/format";
import { priorityById, queueOrder, ticketNumber, type Ticket } from "@/lib/tickets";

/**
 * The queue. Rows are ordered by lib/tickets.queueOrder — breached first, then
 * priority, then oldest — so the top of the list is the next thing to work on,
 * not the newest arrival.
 *
 * One line per ticket on a desktop, two on a phone. Keyboard: ↑/↓ or j/k move
 * between tickets and Enter opens one.
 *
 * Presentational: ordering is pure and tested, and selection is delegated.
 */
export function TicketList({
  tickets,
  customerName,
  assigneeName,
  selectedId,
  showStatus = true,
  now,
  onSelect,
}: {
  tickets: Ticket[];
  customerName: (id: string | null | undefined) => string | null;
  assigneeName: (id: string | null | undefined) => string | null;
  selectedId?: string | null;
  /** Off when every row has the same status, such as on the Open tab. */
  showStatus?: boolean;
  now?: number;
  onSelect: (id: string) => void;
}) {
  const ordered = queueOrder(tickets, now);

  function onKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    const down = event.key === "ArrowDown" || event.key === "j";
    const up = event.key === "ArrowUp" || event.key === "k";
    if (!down && !up) return;
    event.preventDefault();
    const item = event.currentTarget.closest("li");
    const next = (down ? item?.nextElementSibling : item?.previousElementSibling) as HTMLElement | null;
    next?.querySelector("button")?.focus();
  }

  return (
    <ul className="min-h-0 flex-1 overflow-y-auto" aria-label="Tickets">
      {ordered.map((ticket) => {
        const assignee = assigneeName(ticket.assigneeId);
        const customer = customerName(ticket.customerId);
        return (
          <li key={ticket.id}>
            <button
              type="button"
              onClick={() => onSelect(ticket.id)}
              onKeyDown={onKeyDown}
              aria-current={ticket.id === selectedId ? "true" : undefined}
              className={cn(
                "grid w-full grid-cols-[16px_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1 border-b border-border/60 px-4 py-2.5 text-left transition-colors md:h-11 md:py-0 md:pl-5 md:pr-5",
                "focus-visible:bg-primary/[0.06] focus-visible:outline-none",
                ticket.id === selectedId ? "bg-foreground/[0.04]" : "hover:bg-foreground/[0.025]",
              )}
            >
              <PriorityIcon
                priority={ticket.priority}
                title={priorityById(ticket.priority)?.label}
                className="row-span-2 self-start pt-0.5 md:row-span-1 md:self-center md:pt-0"
              />
              <span className="flex min-w-0 items-baseline gap-2.5">
                <span className="tabular hidden w-11 shrink-0 text-[12px] text-muted-foreground md:inline">
                  {ticketNumber(ticket.id, ticket.createdAt)}
                </span>
                <span className="truncate text-[13px] font-medium">{ticket.subject}</span>
                <span className="hidden truncate text-[12.5px] text-muted-foreground lg:inline">
                  {customer ?? ""}
                </span>
              </span>
              <span className="flex shrink-0 items-center gap-2.5 md:gap-3">
                <SlaIndicator ticket={ticket} now={now} />
                {showStatus ? (
                  <StatusBadge status={ticket.status} className="hidden md:inline-flex" />
                ) : null}
                <span className="hidden w-5 md:inline-flex" title={assignee ?? "Unassigned"}>
                  {assignee ? (
                    <Avatar name={assignee} size="sm" />
                  ) : (
                    <span className="size-5 rounded-full border border-dashed border-muted-foreground/50" />
                  )}
                </span>
                <time
                  className="tabular hidden w-14 text-right text-[12px] text-muted-foreground md:inline"
                  dateTime={ticket.createdAt ?? undefined}
                >
                  {relativeTime(ticket.createdAt, now)}
                </time>
              </span>
              <span className="col-start-2 col-end-4 flex min-w-0 items-center gap-2 text-[12px] text-muted-foreground md:hidden">
                <span className="tabular">{ticketNumber(ticket.id, ticket.createdAt)}</span>
                <span aria-hidden="true">·</span>
                <span className="truncate">{customer ?? "No customer"}</span>
                <span className="ml-auto flex shrink-0 items-center gap-1.5">
                  {assignee ? <Avatar name={assignee} size="sm" /> : <span>Unassigned</span>}
                  <time dateTime={ticket.createdAt ?? undefined}>
                    {relativeTime(ticket.createdAt, now)}
                  </time>
                </span>
              </span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/** The queue's shape while the first sync is in flight. */
export function TicketListSkeleton() {
  return (
    <ul className="min-h-0 flex-1 overflow-hidden" aria-hidden="true">
      {Array.from({ length: 10 }).map((_, index) => (
        <li
          key={index}
          className="flex h-14 items-center gap-3 border-b border-border/60 px-4 md:h-11 md:px-5"
        >
          <span className="size-3.5 animate-pulse rounded bg-foreground/[0.07]" />
          <span
            className="h-2.5 animate-pulse rounded-full bg-foreground/[0.07]"
            style={{ width: `${38 + ((index * 17) % 30)}%` }}
          />
          <span className="ml-auto h-2.5 w-12 animate-pulse rounded-full bg-foreground/[0.07]" />
        </li>
      ))}
    </ul>
  );
}
