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
import { Workspace, type ClientRow } from "../workspace";

export function ClientsView({ email }: { email: string }) {
  const [open, setOpen] = useState(false);

  return (
    <Workspace email={email} pathname="/clients">
      {(data) => {
        const forClient = (clientId: string) =>
          data.projects.filter((p) => p.clientId === clientId);

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
            cell: (row) => (
              <span className="text-muted-foreground">{row.email ?? "—"}</span>
            ),
          },
          {
            key: "projects",
            header: "Projects",
            numeric: true,
            cell: (row) => forClient(row.id).length,
          },
          {
            key: "logged",
            header: "Logged",
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
  );
}
