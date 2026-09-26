"use client";

import React, { useEffect, useState } from "react";
import { callFn, useRouter } from "@pylonsync/react";
import { Inbox, Plus, UserX } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/kbd";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { TicketList, TicketListSkeleton } from "@/components/ticket-list";
import { NewTicketDialog } from "@/components/new-ticket-dialog";
import { FilterTabs } from "@/components/filter-tabs";
import {
  DEFAULT_INBOX_TAB,
  INBOX_TABS,
  applyFilter,
  inboxCount,
  inboxFilter,
} from "@/lib/tickets";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "./workspace";

export function InboxView({
  openNew,
  initialFilter,
}: {
  openNew?: boolean;
  initialFilter?: string;
}) {
  const router = useRouter();
  const [dialogOpen, setDialogOpen] = useState(Boolean(openNew));
  const [tab, setTab] = useState<string>(DEFAULT_INBOX_TAB);
  const [unassigned, setUnassigned] = useState(initialFilter === "unassigned");

  // ⌘K "Unassigned open tickets" navigates here with ?filter=unassigned while
  // the inbox may already be mounted.
  useEffect(() => {
    if (initialFilter === "unassigned") {
      setTab(DEFAULT_INBOX_TAB);
      setUnassigned(true);
    }
  }, [initialFilter]);

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
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const visible = applyFilter(data.tickets, inboxFilter(tab, unassigned));

        return (
          <>
            <PageHeader title="Inbox" count={data.navCounts["/"]}>
              <Button size="sm" onClick={() => setDialogOpen(true)} title="New ticket (c)">
                <Plus />
                New ticket
                <Kbd className="ml-0.5 hidden border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground/75 md:inline-flex">
                  C
                </Kbd>
              </Button>
            </PageHeader>

            <FilterTabs
              label="Ticket status"
              value={tab}
              onChange={setTab}
              tabs={INBOX_TABS.map((t) => ({
                id: t.id,
                label: t.label,
                count: inboxCount(data.tickets, t.id, unassigned),
              }))}
              trailing={
                <button
                  type="button"
                  aria-pressed={unassigned}
                  onClick={() => setUnassigned((on) => !on)}
                  title="Show only tickets with no agent"
                  className={
                    "flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-[12.5px] transition-colors " +
                    (unassigned
                      ? "border-primary/40 bg-primary/10 font-medium text-primary"
                      : "border-border text-muted-foreground hover:text-foreground")
                  }
                >
                  <UserX className="size-3.5" />
                  <span className="max-sm:sr-only">Unassigned</span>
                </button>
              }
            />

            {data.loading ? (
              <TicketListSkeleton />
            ) : visible.length === 0 ? (
              <EmptyState
                icon={<Inbox />}
                title={
                  data.tickets.length === 0
                    ? "No tickets yet"
                    : unassigned
                      ? "Every ticket here has an agent"
                      : "No tickets with this status"
                }
                description={
                  data.tickets.length === 0
                    ? "When a customer writes in, their ticket lands here. You can also open one yourself after a phone call."
                    : unassigned
                      ? "Turn off the Unassigned filter to see the tickets your team is working."
                      : "Pick another tab to see the rest of the queue."
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
                showStatus={tab === "all"}
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
    </RequireAuth>
  );
}
