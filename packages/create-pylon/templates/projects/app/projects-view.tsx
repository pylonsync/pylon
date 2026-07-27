"use client";

import React, { useState } from "react";
import { db, useRouter } from "@pylonsync/react";
import { FolderKanban, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import {
  budgetState,
  duration,
  isOpen,
  minutesForProject,
  parseDuration,
  progress,
} from "@/lib/work";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type ProjectRow } from "./workspace";

export function ProjectsView({
  openNew,
}: {
  openNew?: boolean;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(Boolean(openNew));

  return (
    <RequireAuth title="Projects" description="Your team shares one set of projects. Anyone with an account sees it.">
      <Workspace pathname="/">
      {(data) => {
        const columns: ColumnDef<ProjectRow>[] = [
          {
            key: "name",
            header: "Project",
            cell: (row) => <span className="truncate font-medium">{row.name}</span>,
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
            key: "tasks",
            header: "Tasks",
            cell: (row) => {
              const tasks = data.tasks.filter((t) => t.projectId === row.id);
              const p = progress(tasks);
              return (
                <span className="tabular text-muted-foreground">
                  {p.done}/{p.total}
                </span>
              );
            },
          },
          {
            key: "open",
            header: "Open",
            numeric: true,
            cell: (row) =>
              data.tasks.filter((t) => t.projectId === row.id && isOpen(t)).length,
          },
          {
            key: "time",
            header: "Logged",
            numeric: true,
            cell: (row) => {
              const logged = minutesForProject(row.id, data.entries);
              const state = budgetState(row.budgetMinutes, logged);
              return (
                <span
                  className={
                    state === "over"
                      ? "text-destructive"
                      : state === "near"
                        ? "text-stage-proposal"
                        : ""
                  }
                  title={
                    row.budgetMinutes
                      ? `Budget ${duration(row.budgetMinutes)}`
                      : undefined
                  }
                >
                  {duration(logged)}
                </span>
              );
            },
          },
          {
            key: "due",
            header: "Due",
            cell: (row) => (
              <span className="text-muted-foreground">
                {row.dueDate
                  ? new Date(row.dueDate).toLocaleDateString("en-US", {
                      month: "short",
                      day: "numeric",
                    })
                  : "—"}
              </span>
            ),
          },
        ];

        // Active first, then by name — a finished project shouldn\'t sit above
        // the work in flight.
        const rows = [...data.projects].sort((a, b) => {
          const rank = (p: ProjectRow) => (p.status === "active" ? 0 : p.status === "paused" ? 1 : 2);
          return rank(a) - rank(b) || a.name.localeCompare(b.name);
        });

        return (
          <>
            <PageHeader title="Projects" count={rows.length}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                New project
              </Button>
            </PageHeader>

            <DataTable
              rows={rows}
              columns={columns}
              onRowClick={(row) => router.push(`/projects/${row.id}`)}
              empty={
                <EmptyState
                  icon={<FolderKanban />}
                  title={data.loading ? "Loading…" : "No projects yet"}
                  description="A project holds a task board and the time logged against it. Create one to get started."
                  action={
                    <Button size="sm" onClick={() => setOpen(true)}>
                      <Plus />
                      New project
                    </Button>
                  }
                />
              }
            />

            <RecordDialog
              open={open}
              title="New project"
              submitLabel="Create project"
              onOpenChange={setOpen}
              fields={[
                { name: "name", label: "Name", required: true, placeholder: "Dispatch portal" },
                {
                  name: "clientId",
                  label: "Client",
                  options: data.clients
                    .slice()
                    .sort((a, b) => a.name.localeCompare(b.name))
                    .map((c) => ({ value: c.id, label: c.name })),
                },
                { name: "budget", label: "Budget", placeholder: "80h" },
                { name: "rate", label: "Hourly rate", placeholder: "165" },
              ]}
              onCreate={async (values) => {
                // Parsed at the edge — everything below this line is integers.
                await db.insert("Project", {
                  name: values.name,
                  clientId: values.clientId ?? null,
                  status: "active",
                  budgetMinutes: parseDuration(values.budget ?? "") ?? 0,
                  hourlyRateCents: Math.round(Number(values.rate ?? 0) * 100) || 0,
                });
              }}
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
