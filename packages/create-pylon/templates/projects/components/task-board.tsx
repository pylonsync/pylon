import React, { useState } from "react";
import { cn } from "@/lib/utils";
import { TaskCard } from "@/components/task-card";
import { duration, groupByStatus, type Task } from "@/lib/work";

/**
 * The task board.
 *
 * Presentational: it renders whatever tasks it\'s handed and reports a move
 * through `onMove`. The container writes, the write syncs, and the new rows
 * arrive back here — which is also how a teammate\'s move appears, so there is
 * exactly one code path for "a task changed column".
 */
export function TaskBoard({
  tasks,
  assigneeName,
  minutesFor,
  onMove,
  onOpen,
}: {
  tasks: Task[];
  assigneeName: (id: string | null | undefined) => string | null;
  minutesFor: (taskId: string) => number;
  onMove: (taskId: string, status: string) => void;
  onOpen: (taskId: string) => void;
}) {
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const columns = groupByStatus(tasks);

  return (
    <div className="flex h-full gap-3 overflow-x-auto p-4">
      {columns.map((column) => {
        const isTarget = over === column.status.id;
        const logged = column.tasks.reduce(
          (sum, task) => sum + minutesFor(task.id),
          0,
        );
        return (
          <section
            key={column.status.id}
            onDragOver={(event) => {
              // Preventing default is what marks this a valid drop target.
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              if (over !== column.status.id) setOver(column.status.id);
            }}
            onDragLeave={(event) => {
              // Ignore bubbling from children, or the highlight flickers.
              if (event.currentTarget.contains(event.relatedTarget as Node)) return;
              setOver((current) => (current === column.status.id ? null : current));
            }}
            onDrop={(event) => {
              event.preventDefault();
              const id = event.dataTransfer.getData("text/plain") || dragging;
              setOver(null);
              setDragging(null);
              if (!id) return;
              const task = tasks.find((t) => t.id === id);
              // A drop back on the same column is a no-op, not a write.
              if (!task || task.status === column.status.id) return;
              onMove(id, column.status.id);
            }}
            className={cn(
              "flex w-[268px] shrink-0 flex-col rounded-xl border transition-colors",
              isTarget ? "border-ring/60 bg-surface-2/60" : "border-border bg-surface-1/50",
            )}
            aria-label={column.status.label}
          >
            <header className="flex items-center gap-2 px-3 py-2.5">
              <h2 className="text-[12px] font-medium">{column.status.label}</h2>
              <span className="tabular text-[11px] text-muted-foreground">
                {column.tasks.length}
              </span>
              {logged > 0 ? (
                <span className="tabular ml-auto text-[11px] text-muted-foreground">
                  {duration(logged)}
                </span>
              ) : null}
            </header>

            <div className="flex min-h-24 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
              {column.tasks.map((task) => (
                <TaskCard
                  key={task.id}
                  task={task}
                  assignee={assigneeName(task.assigneeId)}
                  loggedMinutes={minutesFor(task.id)}
                  dragging={dragging === task.id}
                  onOpen={onOpen}
                  onDragStart={setDragging}
                  onDragEnd={() => {
                    setDragging(null);
                    setOver(null);
                  }}
                />
              ))}
              {column.tasks.length === 0 ? (
                <p className="px-1 py-6 text-center text-[11px] text-muted-foreground">
                  {isTarget ? "Drop here" : "Nothing here"}
                </p>
              ) : null}
            </div>
          </section>
        );
      })}
    </div>
  );
}
