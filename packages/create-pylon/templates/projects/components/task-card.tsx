import React from "react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { duration, type Task } from "@/lib/work";

/**
 * One task on the board. Draggable, but never the source of truth for which
 * column it sits in — the parent decides that from the synced row, so a
 * teammate\'s move lands here through the same path as your own.
 */
export function TaskCard({
  task,
  assignee,
  loggedMinutes,
  onOpen,
  onDragStart,
  onDragEnd,
  dragging,
}: {
  task: Task;
  assignee?: string | null;
  loggedMinutes: number;
  onOpen: (id: string) => void;
  onDragStart: (id: string) => void;
  onDragEnd: () => void;
  dragging?: boolean;
}) {
  const estimate = Number(task.estimateMinutes) || 0;
  // Only flag an overrun — showing "3h / 8h" on every card is noise, but a task
  // that has blown its estimate is worth seeing from the board.
  const over = estimate > 0 && loggedMinutes > estimate;

  return (
    <article
      draggable
      onDragStart={(event) => {
        // setData is required for Firefox to start a drag at all.
        event.dataTransfer.setData("text/plain", task.id);
        event.dataTransfer.effectAllowed = "move";
        onDragStart(task.id);
      }}
      onDragEnd={onDragEnd}
      onClick={() => onOpen(task.id)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen(task.id);
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={task.title}
      className={cn(
        "group cursor-pointer rounded-lg border border-border bg-card p-2.5 transition-all",
        "hover:border-ring/40 focus-visible:border-ring",
        dragging && "opacity-40",
      )}
    >
      <p className="line-clamp-2 text-[13px] font-medium leading-snug">{task.title}</p>
      <div className="mt-2.5 flex items-center justify-between gap-2">
        {assignee ? <Avatar name={assignee} size="sm" /> : <span />}
        {loggedMinutes > 0 ? (
          <span
            className={cn(
              "tabular text-[11px]",
              over ? "text-destructive" : "text-muted-foreground",
            )}
            title={estimate > 0 ? `Estimate ${duration(estimate)}` : undefined}
          >
            {duration(loggedMinutes)}
            {over ? ` / ${duration(estimate)}` : ""}
          </span>
        ) : null}
      </div>
    </article>
  );
}
