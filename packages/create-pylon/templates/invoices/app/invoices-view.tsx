"use client";

import React, { useEffect, useState } from "react";
import { callFn, useRouter } from "@pylonsync/react";
import { FileText, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/kbd";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { SummaryBar } from "@/components/summary-bar";
import { StatusBadge } from "@/components/status-badge";
import {
  daysOverdue,
  displayStatus,
  money,
  totals,
} from "@/lib/billing";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type InvoiceRow } from "./workspace";

export function InvoicesView({
  openNew,
}: {
  openNew?: boolean;
}) {
  const router = useRouter();
  const [creating, setCreating] = useState(false);

  async function create() {
    if (creating) return;
    setCreating(true);
    try {
      // The number is allocated server-side from the existing series, so the
      // client never invents one.
      const result = await callFn<{ id: string }>("createInvoice", {});
      if (result?.id) router.push(`/invoices/${result.id}`);
    } finally {
      setCreating(false);
    }
  }

  // ?new=invoice from the ⌘K action.
  useEffect(() => {
    if (openNew) void create();
    // Fire once on mount for the deep link; `create` is stable enough here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openNew]);

  // "c" creates an invoice. Ignored while typing.
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
        void create();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [creating]);

  return (
    <RequireAuth title="Invoices" description="Your team shares one set of books. Anyone with an account sees it.">
      <Workspace pathname="/">
      {(data) => {
        const columns: ColumnDef<InvoiceRow>[] = [
          {
            key: "number",
            header: "Invoice",
            cell: (row) => <span className="tabular font-medium">{row.number}</span>,
          },
          {
            key: "client",
            header: "Client",
            cell: (row) => (
              <span className="text-muted-foreground">
                {data.clientName(row.clientId) ?? "—"}
              </span>
            ),
          },
          {
            key: "status",
            header: "Status",
            cell: (row) => {
              const t = totals(row, data.items, data.payments);
              return <StatusBadge status={displayStatus(row, t.balanceCents)} />;
            },
          },
          {
            key: "due",
            header: "Due",
            cell: (row) => {
              const t = totals(row, data.items, data.payments);
              const late = displayStatus(row, t.balanceCents) === "overdue";
              const days = daysOverdue(row);
              if (!row.dueDate) return <span className="text-muted-foreground">—</span>;
              return (
                <span className={late ? "text-destructive" : "text-muted-foreground"}>
                  {late && days !== null
                    ? `${days}d overdue`
                    : new Date(row.dueDate).toLocaleDateString("en-US", {
                        month: "short",
                        day: "numeric",
                      })}
                </span>
              );
            },
          },
          {
            key: "total",
            header: "Total",
            numeric: true,
            cell: (row) => money(totals(row, data.items, data.payments).totalCents),
          },
          {
            key: "balance",
            header: "Balance",
            numeric: true,
            cell: (row) => {
              const t = totals(row, data.items, data.payments);
              return (
                <span className={t.balanceCents > 0 ? "" : "text-muted-foreground"}>
                  {money(t.balanceCents)}
                </span>
              );
            },
          },
        ];

        // Newest first — an invoice list is read from the top, and the number
        // series already encodes the order.
        const rows = [...data.invoices].sort((a, b) =>
          b.number.localeCompare(a.number),
        );

        return (
          <>
            <PageHeader title="Invoices" count={rows.length}>
              <Button size="sm" onClick={() => void create()} disabled={creating}>
                <Plus />
                {creating ? "Creating…" : "New invoice"}
                <Kbd className="ml-1 border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground/70">
                  c
                </Kbd>
              </Button>
            </PageHeader>

            <SummaryBar
              invoices={data.invoices}
              items={data.items}
              payments={data.payments}
            />

            <DataTable
              rows={rows}
              columns={columns}
              onRowClick={(row) => router.push(`/invoices/${row.id}`)}
              empty={
                <EmptyState
                  icon={<FileText />}
                  title={data.loading ? "Loading…" : "No invoices yet"}
                  description="Create one, add the billable lines, then send it. Totals and ageing are derived — there's nothing to keep in sync."
                  action={
                    <Button size="sm" onClick={() => void create()}>
                      <Plus />
                      New invoice
                    </Button>
                  }
                />
              }
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
