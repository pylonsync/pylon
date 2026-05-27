"use client";

/**
 * Pylon Todo — the canonical hello-world.
 *
 * Zero-config, zero-auth demo of Pylon's local-first sync. Each browser
 * silently gets its own guest session via <EnsureGuest>; multi-tab
 * demos still observe live sync without anyone seeing a sign-in screen.
 * Drop something in the list, open another tab, watch it appear.
 *
 * Intentionally uses plain HTML + Tailwind utilities so the demo is
 * obvious end-to-end with no shared-UI plumbing — `db.useQuery<Todo>`
 * + `db.useEntity("Todo")` are the surface, the rest is just markup.
 */
import { useMemo, useRef, useState } from "react";
import { init, db, useSession } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";

const BASE_URL = process.env.NEXT_PUBLIC_PYLON_URL ?? "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "todo-app" });

type Todo = {
  id: string;
  userId: string;
  title: string;
  done: boolean;
  priority: "low" | "med" | "high";
  completedAt?: string | null;
  createdAt: string;
};

type Filter = "all" | "active" | "completed";

const PRIORITIES = [
  { id: "low", label: "Low" },
  { id: "med", label: "Med" },
  { id: "high", label: "High" },
] as const;

const PRIORITY_COLORS: Record<string, string> = {
  low: "bg-emerald-50 text-emerald-700",
  med: "bg-amber-50 text-amber-700",
  high: "bg-rose-50 text-rose-700",
};

export function TodoApp() {
  return (
    <EnsureGuest>
      <List />
    </EnsureGuest>
  );
}

function List() {
  const session = useSession(db.sync);
  const userId = session.userId;

  const queryOpts = useMemo(
    () => ({
      where: userId ? { userId } : undefined,
      orderBy: { createdAt: "desc" as const },
    }),
    [userId],
  );
  const todos = db.useQuery<Todo>("Todo", queryOpts);
  const todoMut = db.useEntity("Todo");
  const [filter, setFilter] = useState<Filter>("all");
  const [draftTitle, setDraftTitle] = useState("");
  const [draftPriority, setDraftPriority] =
    useState<"low" | "med" | "high">("med");
  const inputRef = useRef<HTMLInputElement | null>(null);

  const counts = useMemo(() => {
    const total = todos.data.length;
    const completed = todos.data.filter((t) => t.done).length;
    return { total, completed, active: total - completed };
  }, [todos.data]);

  const filtered = useMemo<Todo[]>(() => {
    if (filter === "active") return todos.data.filter((t) => !t.done);
    if (filter === "completed") return todos.data.filter((t) => t.done);
    return todos.data;
  }, [todos.data, filter]);

  function addTodo(e: React.FormEvent) {
    e.preventDefault();
    const title = draftTitle.trim();
    if (!title || !userId) return;
    todoMut.insert({
      userId,
      title,
      done: false,
      priority: draftPriority,
      createdAt: new Date().toISOString(),
    });
    setDraftTitle("");
    setDraftPriority("med");
    inputRef.current?.focus();
  }

  function toggle(todo: Todo) {
    todoMut.update(todo.id, {
      done: !todo.done,
      completedAt: todo.done ? null : new Date().toISOString(),
    });
  }

  function remove(id: string) {
    todoMut.remove(id);
  }

  function clearCompleted() {
    todos.data
      .filter((t) => t.done)
      .forEach((t) => todoMut.remove(t.id));
  }

  return (
    <div className="mx-auto max-w-2xl px-4 py-10 md:px-6">
      <header className="mb-6">
        <h1 className="text-xl font-semibold tracking-tight">Todos</h1>
        <p className="text-xs text-neutral-500">
          {counts.active === 0
            ? "Inbox zero. Nice."
            : `${counts.active} thing${counts.active === 1 ? "" : "s"} to do`}
        </p>
      </header>

      <form
        onSubmit={addTodo}
        className="mb-4 flex gap-2 rounded-xl border border-neutral-200 bg-white p-2 shadow-sm"
      >
        <input
          ref={inputRef}
          value={draftTitle}
          onChange={(e) => setDraftTitle(e.target.value)}
          placeholder="What needs doing?"
          autoFocus
          className="flex-1 rounded-md border border-transparent bg-transparent px-2 py-1.5 text-sm placeholder:text-neutral-400 focus:outline-none"
        />
        <select
          value={draftPriority}
          onChange={(e) =>
            setDraftPriority(e.target.value as "low" | "med" | "high")
          }
          className="rounded-md border border-neutral-200 bg-white px-2 py-1.5 text-xs text-neutral-700 focus:outline-none"
        >
          {PRIORITIES.map((p) => (
            <option key={p.id} value={p.id}>
              {p.label}
            </option>
          ))}
        </select>
        <button
          type="submit"
          disabled={!draftTitle.trim()}
          className="rounded-md bg-neutral-900 px-3 py-1.5 text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
        >
          Add
        </button>
      </form>

      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-1 rounded-md border border-neutral-200 bg-white p-0.5">
          {(["all", "active", "completed"] as const).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              className={
                "rounded-sm px-3 py-1 text-xs font-medium capitalize transition-colors " +
                (filter === f
                  ? "bg-neutral-900 text-white"
                  : "text-neutral-500 hover:text-neutral-900")
              }
            >
              {f}
              <span className="ml-1.5 font-mono opacity-60">
                {f === "all"
                  ? counts.total
                  : f === "active"
                    ? counts.active
                    : counts.completed}
              </span>
            </button>
          ))}
        </div>
        {counts.completed > 0 ? (
          <button
            type="button"
            onClick={clearCompleted}
            className="text-xs text-neutral-500 hover:text-neutral-900"
          >
            Clear completed
          </button>
        ) : null}
      </div>

      {todos.loading && todos.data.length === 0 ? (
        <p className="rounded-xl border border-neutral-200 bg-white py-12 text-center text-sm text-neutral-500">
          Loading…
        </p>
      ) : filtered.length === 0 ? (
        <p className="rounded-xl border border-neutral-200 bg-white py-12 text-center text-sm text-neutral-500">
          {filter === "completed"
            ? "Nothing finished yet."
            : filter === "active"
              ? "All caught up."
              : "Your list is empty. Type something above and hit Add."}
        </p>
      ) : (
        <ul className="divide-y divide-neutral-100 overflow-hidden rounded-xl border border-neutral-200 bg-white">
          {filtered.map((todo) => (
            <li
              key={todo.id}
              className="group flex items-center gap-3 px-4 py-2.5"
            >
              <input
                type="checkbox"
                checked={todo.done}
                onChange={() => toggle(todo)}
                className="size-4 rounded border-neutral-300"
              />
              <span className="min-w-0 flex-1">
                <span
                  className={
                    "block truncate text-sm " +
                    (todo.done
                      ? "text-neutral-400 line-through"
                      : "text-neutral-900")
                  }
                >
                  {todo.title}
                </span>
                {todo.completedAt ? (
                  <span className="block text-[11px] text-neutral-400">
                    Completed {relativeTime(todo.completedAt)}
                  </span>
                ) : null}
              </span>
              <span
                className={
                  "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide " +
                  PRIORITY_COLORS[todo.priority]
                }
              >
                {todo.priority}
              </span>
              <button
                type="button"
                onClick={() => remove(todo.id)}
                aria-label="Delete todo"
                className="rounded-md px-2 py-1 text-xs text-neutral-400 opacity-0 transition-opacity hover:text-rose-600 group-hover:opacity-100"
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}

      <footer className="mt-6 text-center text-[11px] text-neutral-400">
        Open this page in another tab and watch updates sync live.
      </footer>
    </div>
  );
}

function relativeTime(iso: string) {
  const ms = Date.now() - new Date(iso).getTime();
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s ago`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h ago`;
  return new Date(iso).toLocaleDateString();
}
