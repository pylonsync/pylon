"use client";

/**
 * Pylon Todo — the canonical hello-world.
 *
 * Demonstrates:
 *   - Email/password sign-up + login (`/api/auth/password/*`)
 *   - Live `db.useQuery<Todo>` scoped to the current user
 *   - Optimistic CRUD via `db.useEntity("Todo")` — toggling a checkbox
 *     updates the local store instantly, the server reconciles on the
 *     next sync pull
 *   - A small filter UI (all / active / completed) computed on the
 *     client because the dataset is per-user and small
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  configureClient,
  db,
  storageKey,
} from "@pylonsync/react";
import {
  GripVertical,
  Loader2,
  LogOut,
  Plus,
  SignalHigh,
  SignalLow,
  SignalMedium,
  Trash2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Checkbox } from "@pylonsync/example-ui/checkbox";
import { cn } from "@pylonsync/example-ui/utils";

// Same-origin under native SSR: the Pylon binary serves this app and its API
// on one port, so the client talks to its own origin. Falls back to the dev
// port only during the (never-rendered) server import of this module.
const BASE_URL =
  typeof window !== "undefined"
    ? window.location.origin
    : "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "todo-app" });
configureClient({ baseUrl: BASE_URL, appName: "todo-app" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type Todo = {
  id: string;
  userId: string;
  title: string;
  notes?: string | null;
  done: boolean;
  priority: "low" | "med" | "high";
  dueAt?: string | null;
  completedAt?: string | null;
  createdAt: string;
  /** Manual sort key set when reordering via drag-and-drop. Missing →
   *  row is implicitly sorted by `createdAt`. Floats let inserts
   *  between two adjacent rows reuse the midpoint without renumbering
   *  the whole list. */
  sortKey?: number | null;
};

type AuthState = { token: string; userId: string } | null;

type Filter = "all" | "active" | "completed";

const PRIORITIES: Array<{
  id: "low" | "med" | "high";
  label: string;
  Icon: LucideIcon;
}> = [
  { id: "low", label: "Low", Icon: SignalLow },
  { id: "med", label: "Medium", Icon: SignalMedium },
  { id: "high", label: "High", Icon: SignalHigh },
];

// ---------------------------------------------------------------------------
// Auth helpers
// ---------------------------------------------------------------------------

function readAuth(): AuthState {
  const token = localStorage.getItem(storageKey("token"));
  const userId = localStorage.getItem(storageKey("userId"));
  if (!token || !userId) return null;
  return { token, userId };
}

function saveAuth(token: string, userId: string) {
  localStorage.setItem(storageKey("token"), token);
  localStorage.setItem(storageKey("userId"), userId);
  configureClient({ baseUrl: BASE_URL, appName: "todo-app" });
  window.dispatchEvent(new Event("pylon-auth-changed"));
}

function clearAuth() {
  localStorage.removeItem(storageKey("token"));
  localStorage.removeItem(storageKey("userId"));
  window.dispatchEvent(new Event("pylon-auth-changed"));
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

export function TodoApp() {
  const [auth, setAuth] = useState<AuthState>(() => readAuth());

  useEffect(() => {
    const onChange = () => setAuth(readAuth());
    window.addEventListener("pylon-auth-changed", onChange);
    window.addEventListener("storage", onChange);
    return () => {
      window.removeEventListener("pylon-auth-changed", onChange);
      window.removeEventListener("storage", onChange);
    };
  }, []);

  if (!auth) return <Login />;
  return <List userId={auth.userId} />;
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login() {
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const path =
        mode === "login"
          ? "/api/auth/password/login"
          : "/api/auth/password/register";
      const body =
        mode === "login"
          ? { email, password }
          : { email, password, displayName: name || email.split("@")[0] };
      const res = await fetch(`${BASE_URL}${path}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json.error?.message ?? "auth failed");
      saveAuth(json.token, json.user_id);
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid min-h-screen place-items-center bg-background p-6">
      <Card className="w-full max-w-sm">
        <CardContent className="p-7">
          <h1 className="text-xl font-semibold tracking-tight">
            {mode === "login" ? "Welcome back" : "Create your account"}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {mode === "login"
              ? "Log in to see your todos sync across every tab and device."
              : "10 seconds to create an account. No verification email."}
          </p>

          <form onSubmit={submit} className="mt-5 flex flex-col gap-3">
            {mode === "register" && (
              <Field label="Name">
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Pat Pylon"
                />
              </Field>
            )}
            <Field label="Email">
              <Input
                type="email"
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@example.com"
                autoFocus
              />
            </Field>
            <Field label="Password">
              <Input
                type="password"
                required
                minLength={8}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={mode === "register" ? "8+ characters" : ""}
              />
            </Field>
            {error && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                {error}
              </div>
            )}
            <Button type="submit" disabled={busy} className="mt-1">
              {busy && <Loader2 className="size-4 animate-spin" />}
              {mode === "login" ? "Log in" : "Create account"}
            </Button>
            <div className="pt-1 text-center text-xs text-muted-foreground">
              {mode === "login" ? (
                <>
                  No account?{" "}
                  <button
                    type="button"
                    className="text-primary hover:underline"
                    onClick={() => setMode("register")}
                  >
                    Sign up
                  </button>
                </>
              ) : (
                <>
                  Already registered?{" "}
                  <button
                    type="button"
                    className="text-primary hover:underline"
                    onClick={() => setMode("login")}
                  >
                    Log in
                  </button>
                </>
              )}
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <Label>{label}</Label>
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

function List({ userId }: { userId: string }) {
  const todos = db.useQuery<Todo>("Todo", {
    where: { userId },
    orderBy: { createdAt: "desc" },
  });
  const todoMut = db.useEntity("Todo");
  // Sort by manual sortKey when present, falling back to createdAt
  // for any row that hasn't been reordered yet. Active rows on top,
  // completed at the bottom — toggling done is a soft archive.
  const sorted = useMemo<Todo[]>(() => {
    const rows = todos.data.slice();
    const keyOf = (t: Todo) =>
      typeof t.sortKey === "number" ? t.sortKey : new Date(t.createdAt).getTime();
    rows.sort((a, b) => {
      if (a.done !== b.done) return a.done ? 1 : -1;
      return keyOf(b) - keyOf(a);
    });
    return rows;
  }, [todos.data]);
  const [dragId, setDragId] = useState<string | null>(null);

  function onReorder(draggedId: string, targetId: string, before: boolean) {
    if (draggedId === targetId) return;
    // Find dragged + target in the CURRENT sorted view so the
    // sortKey we mint sits between the right neighbours, not whatever
    // db.useQuery emitted in.
    const idx = sorted.findIndex((t) => t.id === targetId);
    if (idx < 0) return;
    const targetKey =
      typeof sorted[idx]!.sortKey === "number"
        ? sorted[idx]!.sortKey!
        : new Date(sorted[idx]!.createdAt).getTime();
    // The neighbour on the "other side" of the insertion point.
    // before=true → user dropped above target → new key is BIGGER
    // than target (we sort desc by key). before=false → below.
    const neighbourIdx = before ? idx - 1 : idx + 1;
    const neighbour = sorted[neighbourIdx];
    const neighbourKey = neighbour
      ? typeof neighbour.sortKey === "number"
        ? neighbour.sortKey
        : new Date(neighbour.createdAt).getTime()
      : before
        ? targetKey + 1000
        : targetKey - 1000;
    const newKey = (targetKey + neighbourKey) / 2;
    todoMut.update(draggedId, { sortKey: newKey });
  }
  const me = db.useQueryOne<{ displayName?: string; email?: string }>(
    "User",
    userId,
  );
  const [filter, setFilter] = useState<Filter>("all");
  const [draftTitle, setDraftTitle] = useState("");
  const [draftPriority, setDraftPriority] = useState<"low" | "med" | "high">("med");
  const inputRef = useRef<HTMLInputElement>(null);

  const counts = useMemo(() => {
    const total = todos.data.length;
    const completed = todos.data.filter((t) => t.done).length;
    return { total, completed, active: total - completed };
  }, [todos.data]);

  const filtered = useMemo<Todo[]>(() => {
    if (filter === "active") return sorted.filter((t) => !t.done);
    if (filter === "completed") return sorted.filter((t) => t.done);
    return sorted;
  }, [sorted, filter]);

  const addTodo = (e: React.FormEvent) => {
    e.preventDefault();
    const title = draftTitle.trim();
    if (!title) return;
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
  };

  const toggle = (todo: Todo) => {
    todoMut.update(todo.id, {
      done: !todo.done,
      completedAt: todo.done ? null : new Date().toISOString(),
    });
  };

  const remove = (id: string) => todoMut.remove(id);

  const clearCompleted = () => {
    todos.data.filter((t) => t.done).forEach((t) => todoMut.remove(t.id));
  };

  const greet = me.data?.displayName?.split(" ")[0] ?? "you";

  return (
    <div className="mx-auto max-w-2xl px-4 py-10 md:px-6">
      <header className="mb-8 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">
            Hi {greet} 👋
          </h1>
          <p className="text-xs text-muted-foreground">
            {counts.active === 0
              ? "Inbox zero. Nice."
              : `${counts.active} thing${counts.active === 1 ? "" : "s"} to do`}
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => clearAuth()}
          className="text-muted-foreground"
        >
          <LogOut className="size-4" />
          Sign out
        </Button>
      </header>

      <Card>
        <CardContent className="p-3">
          <form onSubmit={addTodo} className="flex gap-2">
            <Input
              ref={inputRef}
              value={draftTitle}
              onChange={(e) => setDraftTitle(e.target.value)}
              placeholder="What needs doing?"
              autoFocus
              className="flex-1"
            />
            <div
              role="radiogroup"
              aria-label="Priority"
              className="inline-flex rounded-md border bg-card p-0.5"
            >
              {PRIORITIES.map(({ id, label, Icon }) => (
                <button
                  key={id}
                  type="button"
                  role="radio"
                  aria-checked={draftPriority === id}
                  aria-label={label}
                  title={label}
                  onClick={() => setDraftPriority(id)}
                  className={cn(
                    "grid size-7 place-items-center rounded-sm transition-colors",
                    draftPriority === id
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <Icon className="size-4" />
                </button>
              ))}
            </div>
            <Button type="submit" disabled={!draftTitle.trim()}>
              <Plus className="size-4" />
              Add
            </Button>
          </form>
        </CardContent>
      </Card>

      <div className="my-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-1 rounded-md border bg-card p-0.5">
          {(["all", "active", "completed"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={cn(
                "rounded-sm px-3 py-1 text-xs font-medium capitalize transition-colors",
                filter === f
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
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
        {counts.completed > 0 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={clearCompleted}
            className="text-muted-foreground"
          >
            Clear completed
          </Button>
        )}
      </div>

      {todos.loading && todos.data.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            Loading…
          </CardContent>
        </Card>
      ) : filtered.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-sm text-muted-foreground">
            {filter === "completed"
              ? "Nothing finished yet."
              : filter === "active"
              ? "All caught up. Add a new todo above."
              : "Your list is empty. Type something above and hit Add."}
          </CardContent>
        </Card>
      ) : (
        <Card className="overflow-hidden">
          <ul className="divide-y divide-border/40">
            {filtered.map((todo) => (
              <Row
                key={todo.id}
                todo={todo}
                onToggle={() => toggle(todo)}
                onRemove={() => remove(todo.id)}
                isDragging={dragId === todo.id}
                onDragStart={() => setDragId(todo.id)}
                onDragEnd={() => setDragId(null)}
                onDropOn={(before) => {
                  if (dragId) onReorder(dragId, todo.id, before);
                  setDragId(null);
                }}
              />
            ))}
          </ul>
        </Card>
      )}

      <footer className="mt-6 text-center text-[11px] text-muted-foreground">
        Open this page in another tab and watch updates sync live.
      </footer>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Row
// ---------------------------------------------------------------------------

const PRIORITY_COLORS: Record<string, string> = {
  low: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  med: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  high: "bg-rose-500/15 text-rose-700 dark:text-rose-300",
};

function Row({
  todo,
  onToggle,
  onRemove,
  isDragging,
  onDragStart,
  onDragEnd,
  onDropOn,
}: {
  todo: Todo;
  onToggle: () => void;
  onRemove: () => void;
  isDragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  /** Called when another row was dropped onto this one. `before=true`
   *  means the cursor was in the top half (drop above), false → bottom. */
  onDropOn: (before: boolean) => void;
}) {
  const [hoverEdge, setHoverEdge] = useState<"top" | "bottom" | null>(null);
  return (
    <li
      draggable
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = "move";
        // Required for Firefox to honor the drag at all.
        e.dataTransfer.setData("text/plain", todo.id);
        onDragStart();
      }}
      onDragEnd={() => {
        setHoverEdge(null);
        onDragEnd();
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        const rect = e.currentTarget.getBoundingClientRect();
        const inTopHalf = e.clientY < rect.top + rect.height / 2;
        setHoverEdge(inTopHalf ? "top" : "bottom");
      }}
      onDragLeave={() => setHoverEdge(null)}
      onDrop={(e) => {
        e.preventDefault();
        const before = hoverEdge === "top";
        setHoverEdge(null);
        onDropOn(before);
      }}
      className={cn(
        "group relative flex items-center gap-3 px-4 py-3 transition-colors hover:bg-accent/40",
        isDragging && "opacity-40",
        hoverEdge === "top" &&
          "before:absolute before:inset-x-2 before:top-0 before:h-0.5 before:rounded-full before:bg-primary",
        hoverEdge === "bottom" &&
          "after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full after:bg-primary",
      )}
    >
      <span
        aria-hidden
        className="cursor-grab text-muted-foreground/50 transition-colors group-hover:text-muted-foreground active:cursor-grabbing"
      >
        <GripVertical className="size-4" />
      </span>
      <Checkbox
        checked={todo.done}
        onCheckedChange={onToggle}
        className="size-5"
      />
      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "truncate text-sm",
            todo.done && "text-muted-foreground line-through",
          )}
        >
          {todo.title}
        </div>
        {todo.completedAt && (
          <div className="text-[11px] text-muted-foreground">
            Completed {relativeTime(todo.completedAt)}
          </div>
        )}
      </div>
      <Badge
        variant="outline"
        className={cn("shrink-0 capitalize", PRIORITY_COLORS[todo.priority])}
      >
        {todo.priority === "med" ? "Medium" : todo.priority}
      </Badge>
      <Button
        variant="ghost"
        size="icon"
        onClick={onRemove}
        aria-label="Delete todo"
        className="size-8 shrink-0 text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
      >
        <Trash2 className="size-4" />
      </Button>
    </li>
  );
}

function relativeTime(iso: string) {
  const ms = Date.now() - new Date(iso).getTime();
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s ago`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
  if (ms < 86_400_000) return `${Math.floor(ms / 3_600_000)}h ago`;
  return new Date(iso).toLocaleDateString();
}
