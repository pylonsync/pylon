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
import { FilterTabs } from "@/components/filter-tabs";
import { Avatar } from "@/components/avatar";
import {
  INVOICE_TABS,
  daysOverdue,
  displayStatus,
  inInvoiceTab,
  invoiceTabCounts,
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
  const [tab, setTab] = useState("all");

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
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const now = Date.now();
        const view = new Map(
          data.invoices.map((invoice) => {
            const t = totals(invoice, data.items, data.payments);
            return [invoice.id, { t, status: displayStatus(invoice, t.balanceCents, now) }] as const;
          }),
        );
        const info = (row: InvoiceRow) => view.get(row.id)!;

        const columns: ColumnDef<InvoiceRow>[] = [
          {
            key: "number",
            header: "Invoice",
            className: "w-[46%] md:w-[130px]",
            cell: (row) => (
              <span className="min-w-0">
                <span className="tabular block truncate font-medium">{row.number}</span>
                <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                  {data.clientName(row.clientId) ?? "No client"}
                </span>
              </span>
            ),
          },
          {
            key: "client",
            header: "Client",
            hideBelow: "md",
            cell: (row) => {
              const name = data.clientName(row.clientId);
              return name ? (
                <span className="flex min-w-0 items-center gap-2.5">
                  <Avatar name={name} size="sm" shape="square" />
                  <span className="truncate">{name}</span>
                </span>
              ) : (
                <span className="text-muted-foreground">—</span>
              );
            },
          },
          {
            key: "status",
            header: "Status",
            className: "w-[104px] md:w-[120px]",
            cell: (row) => <StatusBadge status={info(row).status} />,
          },
          {
            key: "due",
            header: "Due",
            hideBelow: "lg",
            className: "w-[120px]",
            cell: (row) => {
              const late = info(row).status === "overdue";
              const days = daysOverdue(row, now);
              if (!row.dueDate) return <span className="text-muted-foreground">—</span>;
              return (
                <span className={late ? "font-medium text-destructive" : "text-muted-foreground"}>
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
            hideBelow: "sm",
            className: "w-[120px]",
            cell: (row) => money(info(row).t.totalCents),
          },
          {
            key: "balance",
            header: "Balance",
            numeric: true,
            className: "w-[104px] md:w-[120px]",
            cell: (row) => {
              const balance = info(row).t.balanceCents;
              return (
                <span className={balance > 0 ? "font-medium" : "text-muted-foreground"}>
                  {money(balance)}
                </span>
              );
            },
          },
        ];

        const tabCounts = invoiceTabCounts(data.invoices, data.items, data.payments, now);
        // Newest first — an invoice list is read from the top, and the number
        // series already encodes the order.
        const rows = data.invoices
          .filter((invoice) => inInvoiceTab(tab, view.get(invoice.id)!.status))
          .sort((a, b) => b.number.localeCompare(a.number));

        return (
          <>
            <PageHeader title="Invoices" count={data.navCounts["/"]}>
              <Button size="sm" onClick={() => void create()} disabled={creating} title="New invoice (c)">
                <Plus />
                {creating ? "Creating…" : "New invoice"}
                <Kbd className="ml-0.5 hidden border-primary-foreground/25 bg-primary-foreground/10 text-primary-foreground/75 md:inline-flex">
                  C
                </Kbd>
              </Button>
            </PageHeader>

            <SummaryBar
              invoices={data.invoices}
              items={data.items}
              payments={data.payments}
              now={now}
            />

            <FilterTabs
              label="Invoice status"
              value={tab}
              onChange={setTab}
              tabs={INVOICE_TABS.map((t) => ({ id: t.id, label: t.label, count: tabCounts[t.id] }))}
            />

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Invoices"
              onRowClick={(row) => router.push(`/invoices/${row.id}`)}
              empty={
                <EmptyState
                  icon={<FileText />}
                  title={tab === "all" ? "No invoices yet" : "No invoices with this status"}
                  description={
                    tab === "all"
                      ? "Create one, add the billable lines, then send it. Totals and ageing are derived from the lines and payments."
                      : "Pick another tab to see the rest of your invoices."
                  }
                  action={
                    tab === "all" ? (
                      <Button size="sm" onClick={() => void create()}>
                        <Plus />
                        New invoice
                      </Button>
                    ) : undefined
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
