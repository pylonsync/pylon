import React from "react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { PriorityBadge, StatusBadge } from "@/components/priority-badge";
import { SlaIndicator } from "@/components/sla-indicator";
import { relativeTime } from "@/lib/format";
import { queueOrder, ticketNumber, type Ticket } from "@/lib/tickets";

/**
 * The queue. Rows are ordered by lib/tickets.queueOrder — breached first, then
 * priority, then oldest — so the top of the list is genuinely the next thing to
 * work on rather than just the newest arrival.
 *
 * Presentational: ordering is pure and tested, and selection is delegated.
 */
export function TicketList({
  tickets,
  customerName,
  assigneeName,
  selectedId,
  now,
  onSelect,
}: {
  tickets: Ticket[];
  customerName: (id: string | null | undefined) => string | null;
  assigneeName: (id: string | null | undefined) => string | null;
  selectedId?: string | null;
  now?: number;
  onSelect: (id: string) => void;
}) {
  const ordered = queueOrder(tickets, now);

  return (
    <ul className="min-h-0 flex-1 overflow-y-auto">
      {ordered.map((ticket) => {
        const assignee = assigneeName(ticket.assigneeId);
        return (
          <li key={ticket.id}>
            <button
              type="button"
              onClick={() => onSelect(ticket.id)}
              aria-current={ticket.id === selectedId ? "true" : undefined}
              className={cn(
                "flex w-full flex-col gap-1 border-b border-border/60 px-4 py-2.5 text-left transition-colors",
                ticket.id === selectedId ? "bg-surface-2" : "hover:bg-surface-1",
              )}
            >
              <div className="flex items-center gap-2">
                <span className="tabular shrink-0 text-[11px] text-muted-foreground">
                  {ticketNumber(ticket.id, ticket.createdAt)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
                  {ticket.subject}
                </span>
                <SlaIndicator ticket={ticket} now={now} />
              </div>
              <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                <StatusBadge status={ticket.status} />
                <PriorityBadge priority={ticket.priority} />
                <span className="truncate">{customerName(ticket.customerId) ?? "—"}</span>
                <span className="ml-auto flex shrink-0 items-center gap-1.5">
                  {assignee ? (
                    <Avatar name={assignee} size="sm" />
                  ) : (
                    <span className="text-muted-foreground">Unassigned</span>
                  )}
                  <time dateTime={ticket.createdAt ?? undefined}>
                    {relativeTime(ticket.createdAt, now)}
                  </time>
                </span>
              </div>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
