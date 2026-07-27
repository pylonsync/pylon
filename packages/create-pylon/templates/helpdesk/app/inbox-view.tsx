"use client";

import React, { useEffect, useState } from "react";
import { callFn, useRouter } from "@pylonsync/react";
import { Inbox, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/select";
import { Kbd } from "@/components/kbd";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { TicketList } from "@/components/ticket-list";
import { NewTicketDialog } from "@/components/new-ticket-dialog";
import { STATUSES, applyFilter, counts, type Filter } from "@/lib/tickets";
import { Workspace } from "./workspace";

export function InboxView({
  email,
  openNew,
  initialFilter,
}: {
  email: string;
  openNew?: boolean;
  initialFilter?: string;
}) {
  const router = useRouter();
  const [dialogOpen, setDialogOpen] = useState(Boolean(openNew));
  const [status, setStatus] = useState<string>("open");
  const [unassigned, setUnassigned] = useState(initialFilter === "unassigned");

  // "c" opens a ticket, the way an issue tracker does. Ignored while typing so
  // it never swallows a character in the reply box.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if (event.key === "c" && !typing && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        setDialogOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <Workspace email={email} pathname="/">
      {(data) => {
        const filter: Filter = {
          ...(status === "all" ? {} : { status }),
          ...(unassigned ? { unassigned: true } : {}),
        };
        const visible = applyFilter(data.tickets, filter);
        const queue = counts(data.tickets);

        return (
          <>
            <PageHeader title="Inbox" count={visible.length}>
              <div className="w-32">
                <Select
                  aria-label="Status"
                  value={status}
                  onChange={(event) => setStatus(event.target.value)}
                  className="h-7 text-[12px]"
                >
                  <option value="all">All statuses</option>
                  {STATUSES.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </div>
              <Button
                size="sm"
                variant={unassigned ? "default" : "secondary"}
                onClick={() => setUnassigned((on) => !on)}
                title="Show only tickets with no agent"
              >
                Unassigned
                <span className="tabular ml-1 opacity-70">{queue.unassigned}</span>
              </Button>
              <Button size="sm" onClick={() => setDialogOpen(true)}>
                <Plus />
                New ticket
                <Kbd className="ml-1 border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground/70">
                  c
                </Kbd>
              </Button>
            </PageHeader>

            {visible.length === 0 ? (
              <EmptyState
                icon={<Inbox />}
                title={
                  data.loading
                    ? "Loading…"
                    : data.tickets.length === 0
                      ? "No tickets yet"
                      : "Nothing matches this filter"
                }
                description={
                  data.tickets.length === 0
                    ? "When a customer writes in, their ticket lands here. You can also open one yourself after a phone call."
                    : "Try a different status, or clear the unassigned filter."
                }
                action={
                  data.tickets.length === 0 ? (
                    <Button size="sm" onClick={() => setDialogOpen(true)}>
                      <Plus />
                      New ticket
                    </Button>
                  ) : undefined
                }
              />
            ) : (
              <TicketList
                tickets={visible}
                customerName={data.customerName}
                assigneeName={data.agentName}
                onSelect={(id) => router.push(`/tickets/${id}`)}
              />
            )}

            <NewTicketDialog
              open={dialogOpen}
              customers={data.customers}
              onOpenChange={setDialogOpen}
              onCreate={async (draft) => {
                const result = await callFn<{ id: string }>("createTicket", {
                  subject: draft.subject,
                  body: draft.body,
                  priority: draft.priority,
                  ...(draft.customerId ? { customerId: draft.customerId } : {}),
                });
                if (result?.id) router.push(`/tickets/${result.id}`);
              }}
            />
          </>
        );
      }}
    </Workspace>
  );
}
