"use client";

import React, { useState } from "react";
import { callFn, Link } from "@pylonsync/react";
import { ArrowRight, FolderPlus, Sparkles } from "lucide-react";
import { SetupChecklist, type SetupState } from "@/components/setup-checklist";
import {
  Avatar,
  DueLabel,
  EmptyAvatar,
  PriorityIcon,
  Progress,
  ProjectMark,
  StatusIcon,
  shortDay,
} from "@/components/task-ui";
import { FREE_PROJECT_LIMIT } from "@/lib/plans";
import { compareTasks, daysUntil, progressOf, type TaskRow } from "@/lib/tasks";
import { useLiveRows } from "@/lib/use-live";
import type { Project } from "./projects-client";

// The Overview. Projects and tasks start from the server-rendered rows and
// stay live through sync, so a task finished in another tab or by a teammate
// updates the counts here without a reload.
export function Overview({
  tenantId,
  userId,
  firstName,
  projects: initialProjects,
  tasks: initialTasks,
  memberCount,
  plan,
  today,
  canSeed,
  setup,
}: {
  tenantId: string;
  userId: string;
  firstName: string;
  projects: Project[];
  tasks: TaskRow[];
  memberCount: number;
  plan: string;
  /** `YYYY-MM-DD` from the server render. */
  today: string;
  /** Owners and admins may add the sample projects to an empty workspace. */
  canSeed: boolean;
  /** Getting-started state; null once the checklist is dismissed. */
  setup?: SetupState | null;
}) {
  const projects = useLiveRows<Project>("Project", initialProjects, tenantId);
  const tasks = useLiveRows<TaskRow>("Task", initialTasks, tenantId);

  const active = projects.filter((p) => (p.status ?? "active") !== "archived");
  const activeIds = new Set(active.map((p) => p.id));
  const liveTasks = tasks.filter((t) => activeIds.has(t.projectId));
  const open = liveTasks.filter((t) => t.status !== "done");
  const dueThisWeek = open.filter((t) => {
    if (!t.dueDate) return false;
    const d = daysUntil(t.dueDate, today);
    return d >= 0 && d <= 7;
  }).length;
  const overdue = open.filter((t) => t.dueDate && daysUntil(t.dueDate, today) < 0).length;
  const doneThisWeek = liveTasks.filter(
    (t) => t.status === "done" && t.completedAt && daysUntil(t.completedAt.slice(0, 10), today) >= -7,
  ).length;
  const [scope, setScope] = useState<"everyone" | "mine">("everyone");
  const mineCount = open.filter((t) => t.assigneeId === userId).length;
  // Open tasks by due date, earliest first; undated tasks last.
  const upNext = open
    .filter((t) => scope === "everyone" || t.assigneeId === userId)
    .sort((a, b) => (a.dueDate ?? "9999") < (b.dueDate ?? "9999") ? -1 : (a.dueDate ?? "9999") > (b.dueDate ?? "9999") ? 1 : compareTasks(a, b))
    .slice(0, 8);
  const recent = liveTasks
    .filter((t) => t.status === "done" && t.completedAt)
    .sort((a, b) => (a.completedAt! < b.completedAt! ? 1 : -1))
    .slice(0, 4);
  const projectName = new Map(projects.map((p) => [p.id, p.name]));

  const greeting = firstName ? `Welcome back, ${firstName}` : "Welcome back";
  const summary =
    active.length === 0
      ? "Create a project to start tracking work."
      : `${open.length} open ${open.length === 1 ? "task" : "tasks"} across ${active.length} active ${
          active.length === 1 ? "project" : "projects"
        }.`;

  return (
    <div className="mx-auto max-w-[1080px] space-y-8">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-[22px] font-semibold tracking-[-0.015em] text-zinc-900">{greeting}</h1>
          <p className="mt-1 text-[14px] text-zinc-500">{summary}</p>
        </div>
        {active.length > 0 ? (
          <Link
            href="/dashboard/projects"
            className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-zinc-200 bg-white px-3 text-[13px] font-medium text-zinc-700 shadow-[0_1px_1px_rgba(0,0,0,0.03)] transition-colors hover:bg-zinc-50"
          >
            All projects <ArrowRight className="size-3.5" />
          </Link>
        ) : null}
      </div>

      {setup ? <SetupChecklist state={setup} /> : null}

      {projects.length === 0 ? (
        <EmptyWorkspace orgId={tenantId} canSeed={canSeed} />
      ) : (
        <>
          <div className="grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-zinc-200/80 bg-zinc-200/60 lg:grid-cols-4">
            <Stat label="Open tasks" value={open.length} note={`${liveTasks.length - open.length} done`} />
            <Stat
              label="Due in 7 days"
              value={dueThisWeek}
              note={overdue > 0 ? `${overdue} overdue` : "none overdue"}
              alert={overdue > 0}
            />
            <Stat label="Completed this week" value={doneThisWeek} note="last 7 days" />
            <Stat
              label="Plan"
              value={<span className="capitalize">{plan}</span>}
              note={
                plan === "free" ? (
                  <Link href="/dashboard/billing" className="text-brand hover:underline">
                    {active.length} of {FREE_PROJECT_LIMIT} projects used
                  </Link>
                ) : (
                  `${memberCount} ${memberCount === 1 ? "member" : "members"}`
                )
              }
            />
          </div>

          <div className="grid gap-6 lg:grid-cols-[minmax(0,1.55fr)_minmax(0,1fr)]">
            <section className="min-w-0 self-start rounded-xl border border-zinc-200/80 bg-white">
              <header className="flex items-center justify-between border-b border-zinc-100 px-4 py-3">
                <h2 className="text-[13px] font-semibold text-zinc-900">Up next</h2>
                <div className="inline-flex rounded-md bg-zinc-100 p-0.5 text-[12px]">
                  {(
                    [
                      ["everyone", "Everyone"],
                      ["mine", `Assigned to me · ${mineCount}`],
                    ] as const
                  ).map(([key, label]) => (
                    <button
                      key={key}
                      type="button"
                      onClick={() => setScope(key)}
                      className={
                        "rounded px-2 py-0.5 font-medium transition-colors " +
                        (scope === key ? "bg-white text-zinc-900 shadow-[0_1px_2px_rgba(0,0,0,0.08)]" : "text-zinc-500 hover:text-zinc-800")
                      }
                    >
                      {label}
                    </button>
                  ))}
                </div>
              </header>
              {upNext.length === 0 ? (
                <p className="px-4 py-10 text-center text-[13px] text-zinc-500">
                  {scope === "mine" ? "Nothing assigned to you." : "No open tasks."}
                </p>
              ) : (
                <ul className="divide-y divide-zinc-100">
                  {upNext.map((t) => (
                    <li key={t.id}>
                      <Link
                        href={`/dashboard/projects/${t.projectId}`}
                        className="flex items-center gap-3 px-4 py-2.5 transition-colors hover:bg-zinc-50"
                      >
                        <PriorityIcon priority={t.priority} />
                        <StatusIcon status={t.status} />
                        <span className="min-w-0 flex-1 truncate text-[13.5px] text-zinc-800">{t.title}</span>
                        <span className="hidden max-w-[10rem] items-center gap-1.5 truncate text-[12px] text-zinc-500 sm:inline-flex">
                          <ProjectMark
                            id={t.projectId}
                            name={projectName.get(t.projectId) ?? "?"}
                            className="size-4 text-[7.5px]"
                          />
                          <span className="truncate">{projectName.get(t.projectId)}</span>
                        </span>
                        <span className="w-16 text-right">
                          <DueLabel due={t.dueDate} today={today} />
                        </span>
                        {t.assigneeId ? (
                          <Avatar id={t.assigneeId} name={t.assigneeName || "Member"} size="xs" />
                        ) : (
                          <EmptyAvatar size="xs" />
                        )}
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <div className="min-w-0 space-y-6">
            <section className="rounded-xl border border-zinc-200/80 bg-white">
              <header className="flex items-center justify-between border-b border-zinc-100 px-4 py-3">
                <h2 className="text-[13px] font-semibold text-zinc-900">Projects</h2>
                <Link href="/dashboard/projects" className="text-[12px] font-medium text-zinc-500 hover:text-zinc-900">
                  View all
                </Link>
              </header>
              {active.length === 0 ? (
                <p className="px-4 py-10 text-center text-[13px] text-zinc-500">No active projects.</p>
              ) : (
                <ul className="divide-y divide-zinc-100">
                  {active.map((p) => {
                    const pr = progressOf(tasks.filter((t) => t.projectId === p.id));
                    return (
                      <li key={p.id}>
                        <Link
                          href={`/dashboard/projects/${p.id}`}
                          className="block px-4 py-3 transition-colors hover:bg-zinc-50"
                        >
                          <div className="flex items-center gap-2.5">
                            <ProjectMark id={p.id} name={p.name} className="size-6 text-[9.5px]" />
                            <span className="min-w-0 flex-1 truncate text-[13.5px] font-medium text-zinc-900">
                              {p.name}
                            </span>
                            <span className="font-mono text-[11.5px] tabular-nums text-zinc-400">
                              {pr.done}/{pr.total}
                            </span>
                          </div>
                          <Progress percent={pr.percent} className="mt-2.5" />
                        </Link>
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>

            {recent.length > 0 ? (
              <section className="rounded-xl border border-zinc-200/80 bg-white">
                <header className="border-b border-zinc-100 px-4 py-3">
                  <h2 className="text-[13px] font-semibold text-zinc-900">Recently completed</h2>
                </header>
                <ul className="divide-y divide-zinc-100">
                  {recent.map((t) => (
                    <li key={t.id} className="flex items-center gap-2.5 px-4 py-2.5">
                      <StatusIcon status="done" />
                      <span className="min-w-0 flex-1 truncate text-[13px] text-zinc-600">{t.title}</span>
                      {t.assigneeId ? (
                        <Avatar id={t.assigneeId} name={t.assigneeName || "Member"} size="xs" />
                      ) : null}
                      <span className="w-12 text-right text-[12px] tabular-nums text-zinc-400">
                        {shortDay(t.completedAt!.slice(0, 10))}
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            ) : null}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  note,
  alert,
}: {
  label: string;
  value: React.ReactNode;
  note: React.ReactNode;
  alert?: boolean;
}) {
  return (
    <div className="bg-white p-4">
      <div className="text-[12.5px] text-zinc-500">{label}</div>
      <div className="mt-1.5 text-[26px] font-semibold leading-none tracking-[-0.02em] tabular-nums text-zinc-900">
        {value}
      </div>
      <div className={"mt-2 text-[12px] " + (alert ? "text-red-600" : "text-zinc-400")}>{note}</div>
    </div>
  );
}

function EmptyWorkspace({ orgId, canSeed }: { orgId: string; canSeed: boolean }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function seed() {
    setBusy(true);
    setError(null);
    try {
      await callFn("seedWorkspace", { orgId });
    } catch (e) {
      setError(e instanceof Error ? e.message.replace(/^[A-Z_]+:\s*/, "") : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 bg-zinc-50/60 px-6 py-14 text-center">
      <span className="mx-auto flex size-10 items-center justify-center rounded-xl bg-white text-zinc-500 shadow-[0_0_0_1px_rgba(0,0,0,0.06)]">
        <FolderPlus className="size-5" strokeWidth={1.75} />
      </span>
      <h2 className="mt-4 text-[15px] font-semibold text-zinc-900">This workspace has no projects</h2>
      <p className="mx-auto mt-1 max-w-sm text-[13.5px] text-zinc-500">
        Create your first project, or add two sample projects with tasks to try the board.
      </p>
      <div className="mt-5 flex flex-wrap items-center justify-center gap-2">
        <Link
          href="/dashboard/projects?new=1"
          className="inline-flex h-9 items-center rounded-lg bg-zinc-900 px-4 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
        >
          Create a project
        </Link>
        {canSeed ? (
          <button
            type="button"
            onClick={() => void seed()}
            disabled={busy}
            className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-zinc-200 bg-white px-4 text-[13px] font-medium text-zinc-800 transition-colors hover:bg-zinc-50 disabled:opacity-60"
          >
            <Sparkles className="size-3.5 text-zinc-500" strokeWidth={1.75} />
            {busy ? "Adding…" : "Add sample projects"}
          </button>
        ) : null}
      </div>
      {error ? <p className="mt-3 text-[12.5px] text-red-600">{error}</p> : null}
    </div>
  );
}
