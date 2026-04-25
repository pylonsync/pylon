/**
 * Pylon Todo — the canonical hello-world.
 *
 * One file, ~300 lines. Demonstrates:
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
  Check,
  ChevronDown,
  ListTodo,
  Loader2,
  LogOut,
  Plus,
  Trash2,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Checkbox } from "@pylonsync/example-ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@pylonsync/example-ui/select";
import { cn } from "@pylonsync/example-ui/utils";

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
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
};

type AuthState = { token: string; userId: string } | null;

type Filter = "all" | "active" | "completed";

const PRIORITIES = [
  { id: "low", label: "Low" },
  { id: "med", label: "Medium" },
  { id: "high", label: "High" },
] as const;

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
    <div className="grid min-h-screen place-items-center bg-gradient-to-br from-primary/10 via-background to-background p-6">
      <Card className="w-full max-w-sm">
        <CardContent className="p-7">
          <div className="mb-6 flex items-center gap-2">
            <div className="grid size-9 place-items-center rounded-lg bg-primary text-primary-foreground">
              <ListTodo className="size-5" />
            </div>
            <div>
              <div className="font-semibold">Pylon Todo</div>
              <div className="text-xs text-muted-foreground">
                Live, multi-device, per-user todos.
              </div>
            </div>
          </div>

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

  const filtered = useMemo(() => {
    if (filter === "active") return todos.data.filter((t) => !t.done);
    if (filter === "completed") return todos.data.filter((t) => t.done);
    return todos.data;
  }, [todos.data, filter]);

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
        <div className="flex items-center gap-3">
          <div className="grid size-10 place-items-center rounded-lg bg-primary text-primary-foreground">
            <ListTodo className="size-5" />
          </div>
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
            <Select
              value={draftPriority}
              onValueChange={(v) =>
                setDraftPriority(v as "low" | "med" | "high")
              }
            >
              <SelectTrigger className="w-28">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PRIORITIES.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
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
}: {
  todo: Todo;
  onToggle: () => void;
  onRemove: () => void;
}) {
  return (
    <li className="group flex items-center gap-3 px-4 py-3 transition-colors hover:bg-accent/40">
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
