"use client";

import React, { useState } from "react";
import { Link, callFn, db, useRouter } from "@pylonsync/react";
import { ArrowLeft, Clock, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { PageHeader } from "@/components/page-header";
import { EmptyState } from "@/components/empty-state";
import { TaskBoard } from "@/components/task-board";
import { BoardSkeleton } from "@/components/board-skeleton";
import { BudgetBar } from "@/components/budget-bar";
import { TimeDialog } from "@/components/time-dialog";
import { PROJECT_STATUSES, duration, minutesForProject, progress } from "@/lib/work";
import { RequireAuth } from "@/components/require-auth";
import { Workspace } from "../../workspace";

export function ProjectView({
  projectId,
}: {
  projectId: string;
}) {
  const router = useRouter();
  const [newTask, setNewTask] = useState("");
  const [timeFor, setTimeFor] = useState<{ id: string; title: string } | null>(null);

  return (
    <RequireAuth title="Projects" description="Your team shares one set of projects. Anyone with an account sees it.">
      <Workspace pathname="/">
      {(data) => {
        const project = data.projects.find((p) => p.id === projectId);

        if (!project) {
          return (
            <>
              <PageHeader title="Project" />
              <EmptyState
                title={data.loading ? "Loading…" : "Project not found"}
                description={
                  data.loading ? undefined : "It may have been deleted, or the link is wrong."
                }
                action={
                  data.loading ? undefined : (
                    <Button size="sm" variant="secondary" asChild>
                      <Link href="/">Back to projects</Link>
                    </Button>
                  )
                }
              />
            </>
          );
        }

        // Captured after the guard above: TypeScript's narrowing doesn't reach
        // into the nested handler, and widening the check would hide a real bug.
        const current = project;
        const tasks = data.tasks.filter((task) => task.projectId === current.id);
        const logged = minutesForProject(current.id, data.entries);
        const p = progress(tasks);

        async function addTask(event: React.FormEvent) {
          event.preventDefault();
          const title = newTask.trim();
          if (!title) return;
          setNewTask("");
          await db.insert("Task", {
            projectId: current.id,
            title,
            status: "todo",
            position: tasks.filter((t) => t.status === "todo").length,
          });
        }

        return (
          <>
            <PageHeader title={project.name}>
              <span className="tabular text-[12px] text-muted-foreground">
                {p.done}/{p.total} done · {duration(logged)}
              </span>
              <div className="w-28">
                <Select
                  aria-label="Project status"
                  value={project.status}
                  className="h-7 text-[12px]"
                  onChange={(event) =>
                    void db.update("Project", project.id, {
                      status: event.target.value,
                    })
                  }
                >
                  {PROJECT_STATUSES.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </div>
            </PageHeader>

            <div className="flex items-center gap-6 border-b border-border px-4 py-3">
              <Link
                href="/"
                className="inline-flex shrink-0 items-center gap-1.5 text-[12px] text-muted-foreground transition-colors hover:text-foreground"
              >
                <ArrowLeft className="size-3.5" />
                Projects
              </Link>
              <span className="shrink-0 text-[12px] text-muted-foreground">
                {data.clientName(project.clientId) ?? "No client"}
              </span>
              <div className="min-w-0 flex-1 max-w-md">
                <BudgetBar
                  budgetMinutes={project.budgetMinutes}
                  loggedMinutes={logged}
                  hourlyRateCents={project.hourlyRateCents}
                />
              </div>
              <form onSubmit={addTask} className="ml-auto flex shrink-0 items-center gap-2">
                <Input
                  aria-label="New task"
                  placeholder="Add a task…"
                  value={newTask}
                  onChange={(event) => setNewTask(event.target.value)}
                  className="h-7 w-48 text-[12px]"
                />
                <Button type="submit" size="sm" className="h-7" disabled={!newTask.trim()}>
                  <Plus />
                </Button>
              </form>
            </div>

            <div className="min-h-0 flex-1">
              {data.loading ? (
                <BoardSkeleton />
              ) : tasks.length === 0 ? (
                <EmptyState
                  icon={<Clock />}
                  title="No tasks yet"
                  description="Add the first one above. Drag it between columns as it moves, and log time against it from the card."
                />
              ) : (
                <TaskBoard
                  tasks={tasks}
                  assigneeName={data.memberName}
                  minutesFor={(taskId) => data.taskMinutes.get(taskId) ?? 0}
                  onMove={(taskId, status) => {
                    // Through the mutation: the new position is computed
                    // server-side so two simultaneous drags don\'t collide.
                    void callFn("moveTask", { taskId, status });
                  }}
                  onOpen={(taskId) => {
                    const task = tasks.find((t) => t.id === taskId);
                    if (task) setTimeFor({ id: task.id, title: task.title });
                  }}
                />
              )}
            </div>

            <TimeDialog
              open={timeFor !== null}
              taskTitle={timeFor?.title ?? ""}
              onOpenChange={(open) => {
                if (!open) setTimeFor(null);
              }}
              onLog={(minutes, note) =>
                callFn("logTime", {
                  taskId: timeFor?.id ?? "",
                  minutes,
                  ...(note ? { note } : {}),
                })
              }
            />
          </>
        );
      }}
      </Workspace>
    </RequireAuth>
  );
}
