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
import { billableCents, duration, minutesForProject, money } from "@/lib/work";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type ClientRow } from "../workspace";

export function ClientsView() {
  const [open, setOpen] = useState(false);

  return (
    <RequireAuth>
      <Workspace pathname="/clients">
      {(data) => {
        const forClient = (clientId: string) =>
          data.projects.filter((p) => p.clientId === clientId);

        const columns: ColumnDef<ClientRow>[] = [
          {
            key: "name",
            header: "Client",
            className: "w-[60%] md:w-[30%]",
            cell: (row) => (
              <span className="flex min-w-0 items-center gap-2.5">
                <Avatar name={row.name} shape="square" />
                <span className="min-w-0">
                  <span className="block truncate font-medium">{row.name}</span>
                  <span className="block truncate text-[12px] text-muted-foreground md:hidden">
                    {row.email ?? ""}
                  </span>
                </span>
              </span>
            ),
          },
          {
            key: "email",
            header: "Email",
            hideBelow: "md",
            cell: (row) => (
              <span className="text-muted-foreground">{row.email ?? "—"}</span>
            ),
          },
          {
            key: "projects",
            header: "Projects",
            hideBelow: "sm",
            className: "w-[96px]",
            numeric: true,
            cell: (row) => forClient(row.id).length,
          },
          {
            key: "logged",
            header: "Logged",
            hideBelow: "lg",
            className: "w-[110px]",
            numeric: true,
            cell: (row) =>
              duration(
                forClient(row.id).reduce(
                  (sum, p) => sum + minutesForProject(p.id, data.entries),
                  0,
                ),
              ),
          },
          {
            key: "billable",
            header: "Billable",
            className: "w-[120px] md:w-[140px]",
            numeric: true,
            cell: (row) =>
              money(
                forClient(row.id).reduce(
                  (sum, p) =>
                    sum +
                    billableCents(
                      minutesForProject(p.id, data.entries),
                      p.hourlyRateCents,
                    ),
                  0,
                ),
              ),
          },
        ];

        const rows = [...data.clients].sort((a, b) => a.name.localeCompare(b.name));

        return (
          <>
            <PageHeader title="Clients" count={data.navCounts["/clients"]}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                <span className="max-sm:sr-only">New client</span>
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Clients"
              empty={
                <EmptyState
                  icon={<Users />}
                  title="No clients yet"
                  description="Who the work is for. Projects hang off a client, and billable totals roll up here."
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
                { name: "email", label: "Email", type: "email", placeholder: "dana@northwind.co" },
              ]}
              onCreate={async (values) => {
                await db.insert("Client", values);
              }}
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
