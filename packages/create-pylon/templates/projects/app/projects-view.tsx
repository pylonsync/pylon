"use client";

import React, { useState } from "react";
import { db, useRouter } from "@pylonsync/react";
import { FolderKanban, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/page-header";
import { DataTable, type ColumnDef } from "@/components/data-table";
import { EmptyState } from "@/components/empty-state";
import { RecordDialog } from "@/components/record-dialog";
import { FilterTabs } from "@/components/filter-tabs";
import { Meter } from "@/components/meter";
import {
  DEFAULT_PROJECT_TAB,
  PROJECT_STATUSES,
  PROJECT_TABS,
  budgetState,
  duration,
  inProjectTab,
  minutesForProject,
  parseDuration,
  portfolio,
  progress,
} from "@/lib/work";
import { SummaryBar } from "@/components/summary-bar";
import { RequireAuth } from "@/components/require-auth";
import { Workspace, type ProjectRow, type TaskRow } from "./workspace";

export function ProjectsView({
  openNew,
}: {
  openNew?: boolean;
}) {
  const router = useRouter();
  const [open, setOpen] = useState(Boolean(openNew));
  const [tab, setTab] = useState(DEFAULT_PROJECT_TAB);

  return (
    <RequireAuth>
      <Workspace pathname="/">
      {(data) => {
        const tasksByProject = new Map<string, TaskRow[]>();
        for (const task of data.tasks) {
          const list = tasksByProject.get(task.projectId) ?? [];
          list.push(task);
          tasksByProject.set(task.projectId, list);
        }
        const logged = new Map(
          data.projects.map((p) => [p.id, minutesForProject(p.id, data.entries)] as const),
        );

        const columns: ColumnDef<ProjectRow>[] = [
          {
            key: "name",
            header: "Project",
            className: "w-[58%] md:w-[28%]",
            cell: (row) => (
              <span className="min-w-0">
                <span className="flex items-center gap-2">
                  <span className="truncate font-medium">{row.name}</span>
                  {row.status !== "active" && tab === "all" ? (
                    <span className="shrink-0 rounded bg-foreground/[0.06] px-1.5 text-[11px] font-medium text-muted-foreground">
                      {PROJECT_STATUSES.find((s) => s.id === row.status)?.label ?? row.status}
                    </span>
                  ) : null}
                </span>
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
            cell: (row) => (
              <span className="text-muted-foreground">
                {data.clientName(row.clientId) ?? "—"}
              </span>
            ),
          },
          {
            key: "progress",
            header: "Progress",
            className: "w-[42%] md:w-[190px]",
            cell: (row) => {
              const p = progress(tasksByProject.get(row.id) ?? []);
              return (
                <Meter
                  ratio={p.ratio ?? 0}
                  tone="primary"
                  label={`${p.done}/${p.total}`}
                  title={`${p.done} of ${p.total} tasks done`}
                />
              );
            },
          },
          {
            key: "time",
            header: "Budget used",
            hideBelow: "lg",
            className: "w-[220px]",
            cell: (row) => {
              const minutes = logged.get(row.id) ?? 0;
              const state = budgetState(row.budgetMinutes, minutes);
              const budget = Number(row.budgetMinutes) || 0;
              return state === "none" ? (
                <span className="text-muted-foreground">{duration(minutes)} logged</span>
              ) : (
                <Meter
                  ratio={minutes / budget}
                  tone={state === "over" ? "danger" : state === "near" ? "warn" : "neutral"}
                  label={`${duration(minutes)} / ${duration(budget)}`}
                  title={
                    state === "over"
                      ? `Over budget by ${duration(minutes - budget)}`
                      : `${duration(budget - minutes)} left`
                  }
                />
              );
            },
          },
          {
            key: "due",
            header: "Due",
            hideBelow: "sm",
            className: "w-[90px]",
            cell: (row) => (
              <span className="tabular text-muted-foreground">
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

        // Soonest due first, then by name. A project without a date sorts last.
        const rows = data.projects
          .filter((project) => inProjectTab(tab, project))
          .sort(
            (a, b) =>
              (a.dueDate ?? "9999").localeCompare(b.dueDate ?? "9999") ||
              a.name.localeCompare(b.name),
          );

        return (
          <>
            <PageHeader title="Projects" count={data.navCounts["/"]}>
              <Button size="sm" onClick={() => setOpen(true)}>
                <Plus />
                <span className="max-sm:sr-only">New project</span>
              </Button>
            </PageHeader>

            <SummaryBar portfolio={portfolio(data.projects, data.entries)} />

            <FilterTabs
              label="Project status"
              value={tab}
              onChange={setTab}
              tabs={PROJECT_TABS.map((t) => ({
                id: t.id,
                label: t.label,
                count: data.projectCounts[t.id],
              }))}
            />

            <DataTable
              rows={rows}
              columns={columns}
              loading={data.loading}
              label="Projects"
              onRowClick={(row) => router.push(`/projects/${row.id}`)}
              empty={
                <EmptyState
                  icon={<FolderKanban />}
                  title={data.projects.length === 0 ? "No projects yet" : "No projects with this status"}
                  description={
                    data.projects.length === 0
                      ? "A project holds a task board and the time logged against it. Create one to get started."
                      : "Pick another tab to see the rest."
                  }
                  action={
                    data.projects.length === 0 ? (
                      <Button size="sm" onClick={() => setOpen(true)}>
                        <Plus />
                        New project
                      </Button>
                    ) : undefined
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
