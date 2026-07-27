"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { Plus, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { Avatar } from "@/components/avatar";
import { money, totals } from "@/lib/billing";
import { Workspace, type ClientRow } from "../workspace";

export function ClientsView({ email }: { email: string }) {
  const [open, setOpen] = useState(false);

  return (
    <Workspace email={email} pathname="/clients">
      {(data) => {
        // Outstanding per client, derived rather than stored — one less thing
        // to keep in sync when a payment lands.
        const outstanding = (clientId: string) =>
          data.invoices
            .filter((invoice) => invoice.clientId === clientId && invoice.status === "sent")
            .reduce(
              (sum, invoice) =>
                sum + totals(invoice, data.items, data.payments).balanceCents,
              0,
            );

        const columns: ColumnDef<ClientRow>[] = [
          {
            key: "name",
            header: "Client",
            cell: (row) => (
              <span className="flex items-center gap-2">
                <Avatar name={row.name} size="sm" />
                <span className="truncate font-medium">{row.name}</span>
              </span>
            ),
          },
          {
            key: "email",
            header: "Email",
            cell: (row) =>
              row.email ? (
                <a
                  href={`mailto:${row.email}`}
                  onClick={(event) => event.stopPropagation()}
                  className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                >
                  {row.email}
                </a>
              ) : (
                <span className="text-muted-foreground">—</span>
              ),
          },
          {
            key: "invoices",
            header: "Invoices",
            numeric: true,
            cell: (row) =>
              data.invoices.filter((invoice) => invoice.clientId === row.id).length,
          },
          {
            key: "outstanding",
            header: "Outstanding",
            numeric: true,
            cell: (row) => {
              const value = outstanding(row.id);
              return (
                <span className={value > 0 ? "" : "text-muted-foreground"}>
                  {money(value)}
                </span>
              );
            },
          },
        ];

        const rows = [...data.clients].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Clients" count={rows.length}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                New client
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              empty={
                <EmptyState
                  icon={<Users />}
                  title="No clients yet"
                  description="Who you bill. Their address appears on the invoice, so add it once here."
                  action={
                    <Button size="sm" onClick={() => setOpen(true)}>
                      <Plus />
                      New client
                    </Button>
                  }
                />
              }
            />

            <RecordDialog
              open={open}
              title="New client"
              submitLabel="Create client"
              onOpenChange={setOpen}
              fields={[
                { name: "name", label: "Name", required: true, placeholder: "Northwind Logistics" },
                { name: "email", label: "Billing email", type: "email", placeholder: "ap@northwind.co" },
                { name: "address", label: "Address", placeholder: "412 Dock Road, Rotterdam" },
                { name: "taxId", label: "Tax ID", placeholder: "VAT / company number" },
              ]}
              onCreate={async (values) => {
                await db.insert("Client", values);
              }}
            />
          </>
        );
      }}
    </Workspace>
  );
}
