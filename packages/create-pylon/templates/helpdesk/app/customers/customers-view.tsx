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
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type CustomerRow } from "../workspace";

export function CustomersView() {
  const router = useRouter();
  const [open, setOpen] = useState(false);

  return (
    <RequireAuth>
      <Workspace pathname="/customers">
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
            className: "w-[62%] md:w-[24%]",
            cell: (row) => (
              <span className="flex min-w-0 items-center gap-2.5">
                <Avatar name={row.name} />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{row.name}</span>
                  <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                    {row.company ?? row.email}
                  </span>
                </span>
              </span>
            ),
          },
          {
            key: "company",
            header: "Company",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">{row.company ?? "—"}</span>
            ),
          },
          {
            key: "email",
            header: "Email",
            hideBelow: "lg",
            className: "w-[26%]",
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
            header: "Unresolved",
            numeric: true,
            className: "w-[104px]",
            cell: (row) => openCount(row.id),
          },
          {
            key: "total",
            header: "Total",
            numeric: true,
            hideBelow: "sm",
            className: "w-[80px]",
            cell: (row) => totalCount(row.id),
          },
          {
            key: "created",
            header: "Added",
            hideBelow: "xl",
            className: "w-[100px]",
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
            <PageHeader title="Customers" count={data.navCounts["/customers"]}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                <span className="max-sm:sr-only">New customer</span>
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Customers"
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
    </RequireAuth>
  );
}
