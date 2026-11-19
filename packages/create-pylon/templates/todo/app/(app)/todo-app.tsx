"use client";

import React, { useState } from "react";
import { db } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Todo {
  id: string;
  title: string;
  done: boolean;
}

// `<EnsureGuest>` mints a guest session (POST /api/auth/guest) on first load
// if the browser doesn't already have one, so the list works with no login —
// every visitor implicitly becomes their own user and their todos stay
// private. The list itself is gated to the owner by the policy in app.ts.
export function TodoApp() {
  return (
    <EnsureGuest fallback={<ListSkeleton />}>
      <TodoList />
    </EnsureGuest>
  );
}

// `db.useQuery` is a LIVE subscription — it re-renders the instant a Todo is
// added, toggled, or removed, in this tab or another. `db.insert` /
// `db.update` / `db.delete` are OPTIMISTIC: they apply to the local store
// immediately (zero-latency UI) and sync in the background, rolling back
// automatically if a policy rejects the write.
function TodoList() {
  const [title, setTitle] = useState("");
  const { data: todos, loading } = db.useQuery<Todo>("Todo");

  async function addTodo(e: React.FormEvent) {
    e.preventDefault();
    const text = title.trim();
    if (!text) return;
    setTitle("");
    // We don't send userId — `field.owner()` stamps it from the session
    // server-side and rejects any forged value, so this optimistic insert is
    // safe.
    await db.insert("Todo", { title: text, done: false });
  }

  const remaining = todos.filter((t) => !t.done).length;

  return (
    <div className="space-y-5">
      <form onSubmit={addTodo} className="flex items-center gap-2">
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Add a todo…"
          aria-label="Todo"
          autoComplete="off"
          className="flex h-10 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit">Add</Button>
      </form>

      {loading && todos.length === 0 ? (
        <ListSkeleton />
      ) : todos.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          Nothing yet — add a todo above. It appears instantly (optimistic) and
          syncs; open this page in a second tab to watch it arrive live.
        </p>
      ) : (
        <>
          <ul className="space-y-2">
            {todos.map((todo) => (
              <li
                key={todo.id}
                className="flex items-center gap-3 rounded-md border px-3 py-2.5 text-sm"
              >
                <button
                  type="button"
                  aria-label={todo.done ? "Mark not done" : "Mark done"}
                  onClick={() => db.update("Todo", todo.id, { done: !todo.done })}
                  className={
                    "flex size-5 items-center justify-center rounded-full border text-xs transition-colors " +
                    (todo.done
                      ? "border-emerald-600 bg-emerald-600 text-white"
                      : "border-muted-foreground/30 text-transparent hover:border-muted-foreground")
                  }
                >
                  ✓
                </button>
                <span
                  className={
                    todo.done
                      ? "flex-1 text-muted-foreground line-through"
                      : "flex-1"
                  }
                >
                  {todo.title}
                </span>
                <button
                  type="button"
                  aria-label="Delete todo"
                  onClick={() => db.delete("Todo", todo.id)}
                  className="text-muted-foreground/40 transition-colors hover:text-red-600"
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>
          <p className="text-xs text-muted-foreground">
            {remaining} {remaining === 1 ? "todo" : "todos"} left
          </p>
        </>
      )}
    </div>
  );
}

function ListSkeleton() {
  return (
    <ul className="space-y-2" aria-hidden>
      {[0, 1, 2].map((i) => (
        <li
          key={i}
          className="flex items-center gap-3 rounded-md border px-3 py-2.5"
        >
          <span className="size-5 rounded-full bg-muted" />
          <span className="h-4 flex-1 rounded bg-muted" />
        </li>
      ))}
    </ul>
  );
}
