"use client";

import React, { useEffect, useState } from "react";
import { callFn } from "@pylonsync/react";
import { listOrgMembers, type OrgMember } from "@pylonsync/client";
import { CalendarDays, ChevronDown, Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Avatar,
  DueLabel,
  EmptyAvatar,
  PriorityIcon,
  Progress,
  ProjectMark,
  StatusIcon,
} from "@/components/task-ui";
import { personName } from "@/lib/names";
import {
  PRIORITY_LABEL,
  STATUS_LABEL,
  TASK_PRIORITIES,
  TASK_STATUSES,
  compareTasks,
  progressOf,
  type TaskRow,
  type TaskStatus,
} from "@/lib/tasks";
import { useLiveRows } from "@/lib/use-live";
import { errorText, inputCls, type Project } from "./projects-client";

type Member = { userId: string; name: string };

// A project's task board: one column per status. Tasks start from the server
// render and stay live, so a change made in another tab or by a teammate
// moves the card here within a second. Writes go through createTask,
// updateTask, and deleteTask, which check membership against the task's own
// workspace.
export function Board({
  tenantId,
  project,
  tasks: initialTasks,
  today,
}: {
  tenantId: string;
  project: Project;
  tasks: TaskRow[];
  today: string;
}) {
  const all = useLiveRows<TaskRow>("Task", initialTasks, tenantId);
  const tasks = all.filter((t) => t.projectId === project.id);
  const [members, setMembers] = useState<Member[]>([]);
  const [editing, setEditing] = useState<TaskRow | null>(null);
  const [adding, setAdding] = useState<TaskStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pr = progressOf(tasks);

  // Names for the assignee picker. The roster endpoint joins each member's
  // email and name on the server.
  useEffect(() => {
    let alive = true;
    listOrgMembers(tenantId)
      .then((rows: OrgMember[]) => {
        if (!alive) return;
        setMembers(rows.map((m) => ({ userId: m.user_id, name: personName({ name: m.name, email: m.email }) })));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [tenantId]);

  async function setStatus(t: TaskRow, status: string) {
    setError(null);
    try {
      await callFn("updateTask", { taskId: t.id, status });
    } catch (err) {
      setError(errorText(err));
    }
  }

  return (
    <div className="-mx-4 sm:-mx-6 md:-mx-8">
      <div className="flex flex-wrap items-start justify-between gap-4 px-4 sm:px-6 md:px-8">
        <div className="flex min-w-0 items-start gap-3">
          <ProjectMark id={project.id} name={project.name} className="mt-0.5 size-9 text-[12px]" />
          <div className="min-w-0">
            <h1 className="text-[20px] font-semibold tracking-[-0.015em] text-zinc-900">{project.name}</h1>
            {project.description ? (
              <p className="mt-0.5 max-w-xl text-[13.5px] text-zinc-500">{project.description}</p>
            ) : null}
          </div>
        </div>
        <div className="flex w-full items-center gap-3 sm:w-auto">
          <div className="w-full sm:w-56">
            <Progress percent={pr.percent} />
          </div>
          <Button
            size="sm"
            className="h-8 shrink-0 rounded-lg px-3 text-[13px]"
            onClick={() => setAdding("todo")}
          >
            <Plus className="size-4" /> New task
          </Button>
        </div>
      </div>

      {error ? <p className="mt-3 px-4 text-[12.5px] text-red-600 sm:px-6 md:px-8">{error}</p> : null}

      {/* Columns stack below md and sit side by side from md up, scrolling
          sideways inside this box when the panel is narrower than 880px. */}
      <div className="mt-6 pb-2 md:overflow-x-auto">
        <div className="grid grid-cols-1 gap-3 px-4 sm:px-6 md:min-w-[880px] md:grid-cols-4 md:px-8">
          {TASK_STATUSES.map((status) => {
            const col = tasks.filter((t) => t.status === status).sort(compareTasks);
            return (
              <section key={status} className="flex flex-col rounded-xl bg-zinc-50 p-1.5 md:min-h-[320px]">
                <header className="flex items-center gap-2 px-2 pb-2 pt-1.5">
                  <StatusIcon status={status} />
                  <h2 className="text-[13px] font-medium text-zinc-800">{STATUS_LABEL[status]}</h2>
                  <span className="font-mono text-[11.5px] tabular-nums text-zinc-400">{col.length}</span>
                  <button
                    type="button"
                    aria-label={`Add a task to ${STATUS_LABEL[status]}`}
                    onClick={() => setAdding(status)}
                    className="ml-auto flex size-6 items-center justify-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-200/70 hover:text-zinc-700"
                  >
                    <Plus className="size-3.5" />
                  </button>
                </header>
                <ul className="flex flex-col gap-1.5">
                  {col.map((t) => (
                    <li key={t.id}>
                      <TaskCard task={t} today={today} onOpen={() => setEditing(t)} onStatus={(s) => void setStatus(t, s)} />
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
        </div>
      </div>

      <TaskDialog
        open={adding !== null || editing !== null}
        projectId={project.id}
        task={editing ?? undefined}
        initialStatus={adding ?? "todo"}
        members={members}
        onClose={() => {
          setAdding(null);
          setEditing(null);
        }}
      />
    </div>
  );
}

function TaskCard({
  task,
  today,
  onOpen,
  onStatus,
}: {
  task: TaskRow;
  today: string;
  onOpen: () => void;
  onStatus: (status: string) => void;
}) {
  const done = task.status === "done";
  return (
    <div className="group relative rounded-lg bg-white p-3 shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_1px_2px_rgba(0,0,0,0.04)] transition-shadow hover:shadow-[0_0_0_1px_rgba(0,0,0,0.1),0_2px_6px_-2px_rgba(0,0,0,0.08)]">
      <div className="flex items-start gap-2">
        {/* A native select over the status glyph: one tap changes the status
            on desktop and mobile, with the platform's own picker. */}
        <label className="relative z-10 mt-[2px] flex size-4 shrink-0 cursor-pointer items-center justify-center">
          <StatusIcon status={task.status} />
          <select
            aria-label={`Status of ${task.title}`}
            value={task.status}
            onChange={(e) => onStatus(e.target.value)}
            className="absolute inset-0 cursor-pointer opacity-0"
          >
            {TASK_STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_LABEL[s]}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={onOpen}
          className={
            "min-w-0 flex-1 text-left text-[13px] leading-snug after:absolute after:inset-0 after:content-[''] " +
            (done ? "text-zinc-400" : "text-zinc-800")
          }
        >
          {task.title}
        </button>
      </div>
      <div className="mt-2.5 flex items-center gap-2 pl-6">
        <PriorityIcon priority={task.priority} />
        <DueLabel due={task.dueDate} today={today} done={done} />
        <span className="ml-auto">
          {task.assigneeId ? (
            <Avatar id={task.assigneeId} name={task.assigneeName || "Member"} size="xs" />
          ) : (
            <EmptyAvatar size="xs" />
          )}
        </span>
      </div>
    </div>
  );
}

// Create (no `task`) or edit a task. Every field saves through one call.
function TaskDialog({
  open,
  projectId,
  task,
  initialStatus,
  members,
  onClose,
}: {
  open: boolean;
  projectId: string;
  task?: TaskRow;
  initialStatus: TaskStatus;
  members: Member[];
  onClose: () => void;
}) {
  const [title, setTitle] = useState("");
  const [status, setStatus] = useState<string>("todo");
  const [priority, setPriority] = useState<string>("none");
  const [assigneeId, setAssigneeId] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setTitle(task?.title ?? "");
    setStatus(task?.status ?? initialStatus);
    setPriority(task?.priority ?? "none");
    setAssigneeId(task?.assigneeId ?? "");
    setDueDate(task?.dueDate ?? "");
    setError(null);
  }, [open, task, initialStatus]);

  // Keep the current assignee selectable even before the roster loads.
  const options =
    task?.assigneeId && !members.some((m) => m.userId === task.assigneeId)
      ? [{ userId: task.assigneeId, name: task.assigneeName || "Member" }, ...members]
      : members;

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim()) return;
    setBusy(true);
    setError(null);
    try {
      if (task) {
        await callFn("updateTask", { taskId: task.id, title, status, priority, assigneeId, dueDate });
      } else {
        await callFn("createTask", {
          projectId,
          title,
          status,
          priority,
          ...(assigneeId ? { assigneeId } : {}),
          ...(dueDate ? { dueDate } : {}),
        });
      }
      onClose();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!task) return;
    setBusy(true);
    try {
      await callFn("deleteTask", { taskId: task.id });
      onClose();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>{task ? "Edit task" : "New task"}</DialogTitle>
            <DialogDescription className="sr-only">Title, status, priority, assignee, and due date.</DialogDescription>
          </DialogHeader>
          <input
            autoFocus
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Task title"
            aria-label="Task title"
            maxLength={200}
            className={inputCls + " h-10 text-[15px] font-medium"}
          />
          <div className="grid grid-cols-2 gap-3">
            <Field label="Status">
              <Select value={status} onChange={setStatus}>
                {TASK_STATUSES.map((s) => (
                  <option key={s} value={s}>
                    {STATUS_LABEL[s]}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Priority">
              <Select value={priority} onChange={setPriority}>
                {TASK_PRIORITIES.map((p) => (
                  <option key={p} value={p}>
                    {PRIORITY_LABEL[p]}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Assignee">
              <Select value={assigneeId} onChange={setAssigneeId}>
                <option value="">Unassigned</option>
                {options.map((m) => (
                  <option key={m.userId} value={m.userId}>
                    {m.name}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Due date">
              <span className="relative block">
                <input
                  type="date"
                  value={dueDate}
                  onChange={(e) => setDueDate(e.target.value)}
                  className={inputCls + " pl-8"}
                />
                <CalendarDays className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-zinc-400" />
              </span>
            </Field>
          </div>
          {error ? <p className="text-[12.5px] text-red-600">{error}</p> : null}
          <DialogFooter className="sm:justify-between">
            {task ? (
              <button
                type="button"
                onClick={() => void remove()}
                disabled={busy}
                className="inline-flex items-center gap-1.5 text-[13px] font-medium text-zinc-500 transition-colors hover:text-red-600"
              >
                <Trash2 className="size-3.5" /> Delete task
              </button>
            ) : (
              <span />
            )}
            <div className="flex gap-2">
              <Button type="button" variant="outline" size="sm" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" size="sm" disabled={busy || !title.trim()}>
                {busy ? "Saving…" : task ? "Save changes" : "Create task"}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function Select({
  value,
  onChange,
  children,
}: {
  value: string;
  onChange: (value: string) => void;
  children: React.ReactNode;
}) {
  return (
    <span className="relative block">
      <select value={value} onChange={(e) => onChange(e.target.value)} className={inputCls + " appearance-none pr-8"}>
        {children}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-3.5 -translate-y-1/2 text-zinc-400" />
    </span>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[12.5px] font-medium text-zinc-600">{label}</span>
      {children}
    </label>
  );
}
