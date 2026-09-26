"use client";

import React from "react";
import { Link, callFn, useRouter } from "@pylonsync/react";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { Thread } from "@/components/thread";
import { PriorityBadge, StatusBadge } from "@/components/priority-badge";
import { SlaIndicator } from "@/components/sla-indicator";
import { Avatar } from "@/components/avatar";
import { relativeTime } from "@/lib/format";
import { PRIORITIES, STATUSES, isOpen, ticketNumber } from "@/lib/tickets";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function TicketView({
  ticketId,
}: {
  ticketId: string;
}) {
  const router = useRouter();

  return (
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const ticket = data.tickets.find((t) => t.id === ticketId);

        if (!ticket) {
          return (
            <>
              <PageHeader title="Ticket" />
              <EmptyState
                title={data.loading ? "Loading ticket…" : "Ticket not found"}
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
        const openForCustomer = customer
          ? data.tickets.filter((t) => t.customerId === customer.id && isOpen(t)).length
          : 0;

        const setState = (patch: Record<string, string>) =>
          void callFn("setTicketState", { ticketId: ticket.id, ...patch });

        const properties = (
          <>
            <Field label="Status">
              <Select
                aria-label="Status"
                value={ticket.status}
                className="h-8 bg-background md:h-7.5"
                onChange={(event) => setState({ status: event.target.value })}
              >
                {STATUSES.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Priority">
              <Select
                aria-label="Priority"
                value={ticket.priority}
                className="h-8 bg-background md:h-7.5"
                onChange={(event) => setState({ priority: event.target.value })}
              >
                {PRIORITIES.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Assignee">
              <Select
                aria-label="Assignee"
                value={ticket.assigneeId ?? ""}
                className="h-8 bg-background md:h-7.5"
                onChange={(event) => setState({ assigneeId: event.target.value })}
              >
                <option value="">Unassigned</option>
                {data.agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.displayName || agent.email}
                    {agent.id === data.userId ? " (you)" : ""}
                  </option>
                ))}
              </Select>
            </Field>
          </>
        );

        return (
          <>
            <PageHeader
              title={ticket.subject}
              leading={
                <Link
                  href="/"
                  className="hidden items-center gap-1 text-[13px] text-muted-foreground transition-colors hover:text-foreground md:inline-flex"
                >
                  Inbox
                  <ChevronRight className="size-3.5" />
                </Link>
              }
            >
              <SlaIndicator ticket={ticket} />
            </PageHeader>

            <div className="flex min-h-0 flex-1">
              <div className="flex min-w-0 flex-1 flex-col">
                <div className="border-b border-border px-4 py-3 md:px-6 md:py-4">
                  <div className="flex items-center gap-2 text-[12.5px] text-muted-foreground">
                    <span className="tabular">{ticketNumber(ticket.id, ticket.createdAt)}</span>
                    <span aria-hidden="true">·</span>
                    <StatusBadge status={ticket.status} />
                    <PriorityBadge priority={ticket.priority} />
                    <span aria-hidden="true">·</span>
                    <span className="truncate">Opened {relativeTime(ticket.createdAt)}</span>
                  </div>
                  <h2 className="mt-1.5 text-[18px] font-semibold leading-snug tracking-[-0.015em] md:text-[20px]">
                    {ticket.subject}
                  </h2>
                  <dl className="mt-3 grid grid-cols-3 gap-2 lg:hidden">{properties}</dl>
                </div>

                <Thread
                  messages={messages}
                  authorName={data.agentName}
                  customerName={customer?.name ?? null}
                  onSend={(body, internal) =>
                    callFn("replyToTicket", { ticketId: ticket.id, body, internal })
                  }
                />
              </div>

              <aside className="hidden w-[288px] shrink-0 flex-col gap-5 overflow-y-auto border-l border-border bg-surface-1 p-5 lg:flex">
                <dl className="space-y-4">{properties}</dl>

                {customer ? (
                  <section className="border-t border-border pt-5">
                    <h3 className="text-[12px] font-medium text-muted-foreground">Customer</h3>
                    <div className="mt-3 flex items-center gap-2.5">
                      <Avatar name={customer.name} />
                      <div className="min-w-0 leading-tight">
                        <p className="truncate text-[13px] font-medium">{customer.name}</p>
                        {customer.company ? (
                          <p className="truncate text-[12px] text-muted-foreground">
                            {customer.company}
                          </p>
                        ) : null}
                      </div>
                    </div>
                    <a
                      href={`mailto:${customer.email}`}
                      className="mt-3 block truncate text-[12.5px] text-muted-foreground hover:text-foreground"
                    >
                      {customer.email}
                    </a>
                    <p className="mt-1 text-[12.5px] text-muted-foreground">
                      {openForCustomer} open ticket{openForCustomer === 1 ? "" : "s"}
                    </p>
                  </section>
                ) : null}
              </aside>
            </div>
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <dt className="mb-1 text-[11.5px] font-medium text-muted-foreground">{label}</dt>
      <dd className="min-w-0">{children}</dd>
    </div>
  );
}
