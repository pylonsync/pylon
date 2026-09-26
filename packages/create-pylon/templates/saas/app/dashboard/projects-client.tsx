"use client";

import React, { useEffect, useState } from "react";
import { callFn, Link } from "@pylonsync/react";
import { Archive, ArchiveRestore, Check, FolderPlus, Pencil, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Avatar, Progress, ProjectMark } from "@/components/task-ui";
import { FREE_PROJECT_LIMIT, TRIAL_DAYS, planById } from "@/lib/plans";
import { progressOf, type TaskRow } from "@/lib/tasks";
import { useLiveRows } from "@/lib/use-live";

export interface Project {
  id: string;
  orgId: string;
  name: string;
  description?: string | null;
  status?: string;
  createdAt: string;
}

export const inputCls =
  "h-9 w-full rounded-lg border border-zinc-200 bg-white px-3 text-[13.5px] text-zinc-900 shadow-[0_1px_1px_rgba(0,0,0,0.03)] outline-none transition placeholder:text-zinc-400 focus:border-zinc-400 focus:ring-4 focus:ring-zinc-900/5";

/** The server's error message without its `CODE: ` prefix. */
export function errorText(err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err);
  return msg.replace(/^[A-Z_]+:\s*/, "");
}

function isLimitError(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return /LIMIT_REACHED/.test(msg) || (err as { code?: string })?.code === "LIMIT_REACHED";
}

// The Projects page. Rows start from the server render and stay live. Every
// write goes through a server function: createProject and setProjectStatus
// enforce the free plan's cap, deleteProject removes the project's tasks.
// A LIMIT_REACHED answer opens the upgrade dialog.
export function ProjectsPage({
  tenantId,
  projects: initialProjects,
  tasks: initialTasks,
}: {
  tenantId: string;
  projects: Project[];
  tasks: TaskRow[];
}) {
  const projects = useLiveRows<Project>("Project", initialProjects, tenantId);
  const tasks = useLiveRows<TaskRow>("Task", initialTasks, tenantId);
  const [view, setView] = useState<"active" | "archived">("active");
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState<Project | null>(null);
  const [limitHit, setLimitHit] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // `/dashboard/projects?new=1` (from the Overview's empty state) opens the
  // create dialog.
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    if (params.get("new") === "1") {
      setCreating(true);
      window.history.replaceState(null, "", window.location.pathname);
    }
  }, []);

  const archivedCount = projects.filter((p) => p.status === "archived").length;
  const rows = projects
    .filter((p) => ((p.status ?? "active") === "archived" ? "archived" : "active") === view)
    .sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1));

  async function setStatus(p: Project, status: "active" | "archived") {
    setError(null);
    try {
      await callFn("setProjectStatus", { projectId: p.id, status });
    } catch (err) {
      if (isLimitError(err)) setLimitHit(true);
      else setError(errorText(err));
    }
  }

  async function remove(p: Project) {
    const count = tasks.filter((t) => t.projectId === p.id).length;
    const detail = count > 0 ? ` and its ${count} ${count === 1 ? "task" : "tasks"}` : "";
    if (!window.confirm(`Delete ${p.name}${detail}? This can't be undone.`)) return;
    setError(null);
    try {
      await callFn("deleteProject", { projectId: p.id });
    } catch (err) {
      setError(errorText(err));
    }
  }

  return (
    <div className="mx-auto max-w-[1080px]">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="inline-flex rounded-lg bg-zinc-100 p-0.5 text-[13px]">
          {(["active", "archived"] as const).map((v) => (
            <button
              key={v}
              type="button"
              onClick={() => setView(v)}
              className={
                "rounded-md px-3 py-1 font-medium transition-colors " +
                (view === v ? "bg-white text-zinc-900 shadow-[0_1px_2px_rgba(0,0,0,0.08)]" : "text-zinc-500 hover:text-zinc-800")
              }
            >
              {v === "active" ? "Active" : "Archived"}
              {v === "archived" && archivedCount > 0 ? (
                <span className="ml-1.5 font-mono text-[11px] text-zinc-400">{archivedCount}</span>
              ) : null}
            </button>
          ))}
        </div>
        <Button size="sm" onClick={() => setCreating(true)} className="h-8 rounded-lg px-3 text-[13px]">
          <Plus className="size-4" /> New project
        </Button>
      </div>

      {error ? <p className="mt-3 text-[12.5px] text-red-600">{error}</p> : null}

      {rows.length === 0 ? (
        <div className="mt-5 flex flex-col items-center gap-2 rounded-xl border border-dashed border-zinc-300 bg-zinc-50/60 py-14 text-center">
          <span className="flex size-10 items-center justify-center rounded-xl bg-white text-zinc-500 shadow-[0_0_0_1px_rgba(0,0,0,0.06)]">
            <FolderPlus className="size-5" strokeWidth={1.75} />
          </span>
          <p className="mt-2 text-[14px] font-medium text-zinc-800">
            {view === "active" ? "No active projects" : "No archived projects"}
          </p>
          <p className="text-[13px] text-zinc-500">
            {view === "active" ? "Create a project to start adding tasks." : "Archived projects show up here."}
          </p>
        </div>
      ) : (
        <div className="mt-5 overflow-hidden rounded-xl border border-zinc-200/80 bg-white">
          <div className="hidden grid-cols-[minmax(0,1fr)_200px_96px_80px_92px] items-center gap-6 border-b border-zinc-100 bg-zinc-50/60 px-4 py-2 text-[12px] font-medium text-zinc-500 md:grid">
            <span>Name</span>
            <span>Progress</span>
            <span>Team</span>
            <span className="text-right">Open</span>
            <span />
          </div>
          <ul className="divide-y divide-zinc-100">
            {rows.map((p) => {
              const own = tasks.filter((t) => t.projectId === p.id);
              const pr = progressOf(own);
              const archived = p.status === "archived";
              return (
                <li
                  key={p.id}
                  className="group grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-2 px-4 py-3 transition-colors hover:bg-zinc-50/70 md:grid-cols-[minmax(0,1fr)_200px_96px_80px_92px] md:gap-6"
                >
                  <Link href={`/dashboard/projects/${p.id}`} className="flex min-w-0 items-center gap-3">
                    <ProjectMark id={p.id} name={p.name} className="size-8 text-[11px]" />
                    <span className="min-w-0">
                      <span className="block truncate text-[14px] font-medium text-zinc-900">{p.name}</span>
                      <span className="block truncate text-[12.5px] text-zinc-500">
                        {p.description || "No description"}
                      </span>
                    </span>
                  </Link>
                  <Progress percent={pr.percent} className="col-span-2 row-start-2 md:col-span-1 md:row-start-auto" />
                  <span className="hidden -space-x-1 md:flex">
                    {team(own).map((m) => (
                      <span key={m.id} className="rounded-full ring-2 ring-white">
                        <Avatar id={m.id} name={m.name} size="xs" />
                      </span>
                    ))}
                  </span>
                  <span className="hidden text-right font-mono text-[12.5px] tabular-nums text-zinc-600 md:block">
                    {pr.total - pr.done}
                  </span>
                  <div className="col-start-2 row-start-1 flex items-center justify-end gap-0.5 md:col-start-auto md:row-start-auto md:opacity-0 md:transition-opacity md:group-hover:opacity-100 md:group-focus-within:opacity-100">
                    <IconBtn label="Edit project" onClick={() => setEditing(p)}>
                      <Pencil className="size-3.5" />
                    </IconBtn>
                    <IconBtn
                      label={archived ? "Restore project" : "Archive project"}
                      onClick={() => void setStatus(p, archived ? "active" : "archived")}
                    >
                      {archived ? <ArchiveRestore className="size-3.5" /> : <Archive className="size-3.5" />}
                    </IconBtn>
                    <IconBtn label="Delete project" danger onClick={() => void remove(p)}>
                      <Trash2 className="size-3.5" />
                    </IconBtn>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}

      <ProjectDialog
        open={creating}
        onClose={() => setCreating(false)}
        orgId={tenantId}
        onLimit={() => {
          setCreating(false);
          setLimitHit(true);
        }}
      />
      <ProjectDialog
        open={editing !== null}
        project={editing ?? undefined}
        onClose={() => setEditing(null)}
        orgId={tenantId}
        onLimit={() => setLimitHit(true)}
      />
      <UpgradeDialog open={limitHit} onClose={() => setLimitHit(false)} />
    </div>
  );
}

/** Distinct assignees of a project's tasks, most tasks first, at most four. */
function team(tasks: TaskRow[]): { id: string; name: string }[] {
  const counts = new Map<string, { id: string; name: string; n: number }>();
  for (const t of tasks) {
    if (!t.assigneeId) continue;
    const cur = counts.get(t.assigneeId) ?? { id: t.assigneeId, name: t.assigneeName || "Member", n: 0 };
    cur.n++;
    counts.set(t.assigneeId, cur);
  }
  return [...counts.values()].sort((a, b) => b.n - a.n).slice(0, 4);
}

function IconBtn({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={
        "flex size-7 items-center justify-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-100 " +
        (danger ? "hover:text-red-600" : "hover:text-zinc-800")
      }
    >
      {children}
    </button>
  );
}

// Create (no `project`) or edit a project's name and description. Creates go
// through createProject (cap-checked); edits through updateProject.
function ProjectDialog({
  open,
  project,
  orgId,
  onClose,
  onLimit,
}: {
  open: boolean;
  project?: Project;
  orgId: string;
  onClose: () => void;
  onLimit: () => void;
}) {
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setName(project?.name ?? "");
    setDesc(project?.description ?? "");
    setError(null);
  }, [open, project]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true);
    setError(null);
    try {
      if (project) {
        await callFn("updateProject", {
          projectId: project.id,
          name: name.trim(),
          description: desc.trim() || undefined,
        });
      } else {
        await callFn("createProject", {
          orgId,
          name: name.trim(),
          ...(desc.trim() ? { description: desc.trim() } : {}),
        });
      }
      onClose();
    } catch (err) {
      if (isLimitError(err)) onLimit();
      else setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>{project ? "Edit project" : "New project"}</DialogTitle>
            <DialogDescription>
              {project ? "Change the name or description." : "Projects hold the tasks your team works on."}
            </DialogDescription>
          </DialogHeader>
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">Name</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Website relaunch"
              maxLength={80}
              className={inputCls}
            />
          </label>
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">Description</span>
            <textarea
              value={desc}
              onChange={(e) => setDesc(e.target.value)}
              rows={3}
              placeholder="What the project delivers, and by when"
              className={inputCls + " h-auto resize-none py-2"}
            />
          </label>
          {error ? <p className="text-[12.5px] text-red-600">{error}</p> : null}
          <DialogFooter>
            <Button type="button" variant="outline" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" size="sm" disabled={busy || !name.trim()}>
              <Check className="size-3.5" />
              {busy ? "Saving…" : project ? "Save changes" : "Create project"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// Shown when a server function answers LIMIT_REACHED. One action: go to
// Billing, where the trial starts.
export function UpgradeDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const pro = planById("pro")!;
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>The free plan allows {FREE_PROJECT_LIMIT} active projects</DialogTitle>
          <DialogDescription>
            Archive a project, or upgrade to Pro for unlimited projects. Pro starts with a {TRIAL_DAYS}-day
            free trial.
          </DialogDescription>
        </DialogHeader>
        <ul className="space-y-1.5 text-[13.5px] text-zinc-700">
          {pro.features.map((f) => (
            <li key={f} className="flex items-center gap-2">
              <Check className="size-3.5 text-brand" /> {f}
            </li>
          ))}
        </ul>
        <DialogFooter>
          <Button type="button" variant="outline" size="sm" onClick={onClose}>
            Not now
          </Button>
          <a
            href="/dashboard/billing?upgrade=pro&interval=annual"
            className="inline-flex h-8 items-center rounded-md bg-zinc-900 px-3 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
          >
            {pro.cta}
          </a>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
