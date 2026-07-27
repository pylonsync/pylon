"use client";

import React, { useState } from "react";
import { db, useRouter } from "@pylonsync/react";
import { Plus, Users } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { Avatar } from "@/components/avatar";
import { relativeTime } from "@/lib/format";
import { isOpen } from "@/lib/tickets";
import { Workspace, type CustomerRow } from "../workspace";

export function CustomersView({ email }: { email: string }) {
  const router = useRouter();
  const [open, setOpen] = useState(false);

  return (
    <Workspace email={email} pathname="/customers">
      {(data) => {
        // Ticket counts per customer, derived rather than stored — one less
        // thing to keep in sync when a ticket is opened or solved.
        const openCount = (customerId: string) =>
          data.tickets.filter((t) => t.customerId === customerId && isOpen(t)).length;
        const totalCount = (customerId: string) =>
          data.tickets.filter((t) => t.customerId === customerId).length;

        const columns: ColumnDef<CustomerRow>[] = [
          {
            key: "name",
            header: "Customer",
            cell: (row) => (
              <span className="flex items-center gap-2">
                <Avatar name={row.name} size="sm" />
                <span className="truncate font-medium">{row.name}</span>
              </span>
            ),
          },
          {
            key: "company",
            header: "Company",
            cell: (row) => (
              <span className="text-muted-foreground">{row.company ?? "—"}</span>
            ),
          },
          {
            key: "email",
            header: "Email",
            cell: (row) => (
              <a
                href={`mailto:${row.email}`}
                onClick={(event) => event.stopPropagation()}
                className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              >
                {row.email}
              </a>
            ),
          },
          {
            key: "open",
            header: "Open",
            numeric: true,
            cell: (row) => openCount(row.id),
          },
          {
            key: "total",
            header: "Total",
            numeric: true,
            cell: (row) => totalCount(row.id),
          },
          {
            key: "created",
            header: "Added",
            cell: (row) => (
              <span className="text-muted-foreground">
                {relativeTime(row.createdAt)}
              </span>
            ),
          },
        ];

        const rows = [...data.customers].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Customers" count={rows.length}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                New customer
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              empty={
                <EmptyState
                  icon={<Users />}
                  title="No customers yet"
                  description="The people who write in. Add one so a ticket can be attributed to a real account."
                  action={
                    <Button size="sm" onClick={() => setOpen(true)}>
                      <Plus />
                      New customer
                    </Button>
                  }
                />
              }
            />

            <RecordDialog
              open={open}
              title="New customer"
              submitLabel="Create customer"
              onOpenChange={setOpen}
              fields={[
                { name: "name", label: "Name", required: true, placeholder: "Dana Whitfield" },
                {
                  name: "email",
                  label: "Email",
                  type: "email",
                  required: true,
                  placeholder: "dana@northwind.co",
                },
                { name: "company", label: "Company", placeholder: "Northwind Logistics" },
              ]}
              onCreate={async (values) => {
                // A plain policy-checked insert — nothing to validate beyond
                // the field types, and nothing to stamp the policy doesn't gate.
                await db.insert("Customer", values);
              }}
            />
          </>
        );
      }}
    </Workspace>
  );
}
