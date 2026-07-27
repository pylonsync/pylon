"use client";

import React from "react";
import { Link, callFn, useRouter } from "@pylonsync/react";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { Thread } from "@/components/thread";
import { PriorityBadge } from "@/components/priority-badge";
import { SlaIndicator } from "@/components/sla-indicator";
import { Avatar } from "@/components/avatar";
import { relativeTime } from "@/lib/format";
import { PRIORITIES, STATUSES, ticketNumber } from "@/lib/tickets";
import { Workspace } from "../../workspace";

export function TicketView({
  email,
  ticketId,
}: {
  email: string;
  ticketId: string;
}) {
  const router = useRouter();

  return (
    <Workspace email={email} pathname="/">
      {(data) => {
        const ticket = data.tickets.find((t) => t.id === ticketId);

        if (!ticket) {
          return (
            <>
              <PageHeader title="Ticket" />
              <EmptyState
                title={data.loading ? "Loading…" : "Ticket not found"}
                description={
                  data.loading
                    ? undefined
                    : "It may have been deleted, or the link is wrong."
                }
                action={
                  data.loading ? undefined : (
                    <Button size="sm" variant="secondary" asChild>
                      <Link href="/">Back to inbox</Link>
                    </Button>
                  )
                }
              />
            </>
          );
        }

        const customer = data.customers.find((c) => c.id === ticket.customerId);
        const messages = data.messages.filter((m) => m.ticketId === ticket.id);

        return (
          <>
            <PageHeader title={ticket.subject}>
              <SlaIndicator ticket={ticket} />
              <div className="w-28">
                <Select
                  aria-label="Priority"
                  value={ticket.priority}
                  className="h-7 text-[12px]"
                  onChange={(event) =>
                    void callFn("setTicketState", {
                      ticketId: ticket.id,
                      priority: event.target.value,
                    })
                  }
                >
                  {PRIORITIES.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </Select>
              </div>
              <div className="w-28">
                <Select
                  aria-label="Status"
                  value={ticket.status}
                  className="h-7 text-[12px]"
                  onChange={(event) =>
                    void callFn("setTicketState", {
                      ticketId: ticket.id,
                      status: event.target.value,
                    })
                  }
                >
                  {STATUSES.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </div>
            </PageHeader>

            <div className="flex items-center gap-3 border-b border-border px-6 py-2.5 text-[12px] text-muted-foreground">
              <Link
                href="/"
                className="inline-flex items-center gap-1.5 transition-colors hover:text-foreground"
              >
                <ArrowLeft className="size-3.5" />
                Inbox
              </Link>
              <span aria-hidden="true">·</span>
              <span className="tabular">
                {ticketNumber(ticket.id, ticket.createdAt)}
              </span>
              <span aria-hidden="true">·</span>
              <PriorityBadge priority={ticket.priority} />
              {customer ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="flex items-center gap-1.5">
                    <Avatar name={customer.name} size="sm" />
                    {customer.name}
                    {customer.company ? ` · ${customer.company}` : ""}
                  </span>
                </>
              ) : null}
              <span aria-hidden="true">·</span>
              <span>Opened {relativeTime(ticket.createdAt)}</span>

              <div className="ml-auto w-40">
                <Select
                  aria-label="Assignee"
                  value={ticket.assigneeId ?? ""}
                  className="h-7 text-[12px]"
                  onChange={(event) =>
                    void callFn("setTicketState", {
                      ticketId: ticket.id,
                      assigneeId: event.target.value,
                    })
                  }
                >
                  <option value="">Unassigned</option>
                  {data.agents.map((agent) => (
                    <option key={agent.id} value={agent.id}>
                      {agent.displayName || agent.email}
                    </option>
                  ))}
                </Select>
              </div>
            </div>

            <Thread
              messages={messages}
              authorName={data.agentName}
              customerName={customer?.name ?? null}
              onSend={(body, internal) =>
                callFn("replyToTicket", { ticketId: ticket.id, body, internal })
              }
            />
          </>
        );
      }}
    </Workspace>
  );
}
