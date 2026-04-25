/**
 * Crew — a multi-agent orchestration console.
 *
 * Demonstrates streaming actions: each run produces incremental Message
 * updates that sync to every connected client via the change log, so the
 * UI watches the agent "think" in real time without app-specific sockets.
 */
import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
  useSession,
} from "@pylonsync/react";
import {
  Bot,
  ChevronRight,
  ListTree,
  LogOut,
  Play,
  Plus,
  Sparkles,
  Workflow,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { Badge } from "@pylonsync/example-ui/badge";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@pylonsync/example-ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@pylonsync/example-ui/select";
import { cn } from "@pylonsync/example-ui/utils";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "crew" });
configureClient({ baseUrl: BASE_URL, appName: "crew" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type User = {
  id: string;
  email: string;
  displayName: string;
  avatarColor: string;
};
type Organization = {
  id: string;
  name: string;
  slug: string;
  createdBy: string;
  createdAt: string;
};
type OrgMember = {
  id: string;
  userId: string;
  orgId: string;
  role: string;
  joinedAt: string;
};
type Agent = {
  id: string;
  orgId: string;
  name: string;
  role: string;
  systemPrompt: string;
  model: string;
  avatarEmoji: string;
  createdAt: string;
  createdBy: string;
};
type Pipeline = {
  id: string;
  orgId: string;
  name: string;
  description?: string | null;
  createdBy: string;
  createdAt: string;
};
type PipelineStep = {
  id: string;
  orgId: string;
  pipelineId: string;
  position: number;
  agentId: string;
  instruction: string;
};
type Run = {
  id: string;
  orgId: string;
  pipelineId?: string | null;
  agentId?: string | null;
  title: string;
  input: string;
  status: string;
  startedBy: string;
  createdAt: string;
  startedAt?: string | null;
  completedAt?: string | null;
  error?: string | null;
  tokensIn?: number | null;
  tokensOut?: number | null;
};
type RunStep = {
  id: string;
  orgId: string;
  runId: string;
  stepNumber: number;
  agentId: string;
  input: string;
  output: string;
  status: string;
  tokensIn?: number | null;
  tokensOut?: number | null;
  startedAt?: string | null;
  completedAt?: string | null;
  error?: string | null;
};
type Message = {
  id: string;
  orgId: string;
  runId: string;
  runStepId: string;
  role: string;
  content: string;
  createdAt: string;
};

type Page = "agents" | "pipelines" | "runs";

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

export function CrewApp() {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as User) : null;
    } catch {
      return null;
    }
  });
  const { tenantId: activeOrgId } = useSession(db.sync);
  const [page, setPage] = useState<Page>("runs");
  const [activeRunId, setActiveRunId] = useState<string | null>(null);

  async function signOut() {
    const token = localStorage.getItem(storageKey("token"));
    localStorage.removeItem(storageKey("token"));
    localStorage.removeItem(storageKey("user"));
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try {
      indexedDB.deleteDatabase("pylon_sync_crew");
    } catch {}
    setCurrentUser(null);
    await db.sync.notifySessionChanged();
  }

  async function selectOrg(orgId: string | null) {
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    const res = await fetch(`${BASE_URL}/api/auth/select-org`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ orgId }),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error?.message || `switch failed (${res.status})`);
    }
    await db.sync.notifySessionChanged();
  }

  if (!currentUser) return <Login onReady={setCurrentUser} />;

  return (
    <OrgGate
      currentUser={currentUser}
      activeOrgId={activeOrgId}
      onSelectOrg={selectOrg}
      onSignOut={signOut}
    >
      {(org) => (
        <Workspace
          org={org}
          currentUser={currentUser}
          page={page}
          setPage={setPage}
          activeRunId={activeRunId}
          setActiveRunId={setActiveRunId}
          onSignOut={signOut}
        />
      )}
    </OrgGate>
  );
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [email, setEmail] = useState("captain@crew.dev");
  const [name, setName] = useState("Captain");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go(e?: React.FormEvent) {
    e?.preventDefault();
    setLoading(true);
    setErr(null);
    try {
      const session = await fetch(`${BASE_URL}/api/auth/guest`, {
        method: "POST",
      }).then((r) => r.json());
      const token: string = session.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL, appName: "crew" });
      const user = await callFn<User>("upsertUser", { email, displayName: name });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ user_id: user.id }),
      });
      localStorage.setItem(storageKey("user"), JSON.stringify(user));
      void db.sync.notifySessionChanged();
      onReady(user);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="grid h-screen lg:grid-cols-2">
      <div className="flex items-center justify-center p-10">
        <div className="w-full max-w-sm">
          <BrandRow className="mb-6" />
          <h1 className="mb-1 text-3xl font-bold tracking-tight">Sign in</h1>
          <p className="mb-6 text-sm leading-relaxed text-muted-foreground">
            Orchestrate AI agents. Chain them into pipelines. Watch runs stream live.
          </p>
          <form onSubmit={go} className="flex flex-col gap-3">
            <Field label="Email">
              <Input
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@crew.dev"
                autoFocus
              />
            </Field>
            <Field label="Display name">
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Captain"
              />
            </Field>
            {err && (
              <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
                {err}
              </div>
            )}
            <Button type="submit" disabled={loading} className="mt-2">
              {loading ? "Signing in…" : "Continue"}
            </Button>
          </form>
        </div>
      </div>
      <div className="hidden bg-gradient-to-br from-primary/30 via-primary/10 to-background lg:flex lg:items-center lg:p-12">
        <div className="max-w-md">
          <h2 className="mb-4 text-3xl font-semibold leading-tight">
            A console for
            <br />
            your agent crew.
          </h2>
          <p className="text-sm leading-relaxed text-muted-foreground">
            Define specialized agents. Compose them into pipelines. Stream their
            output in real time. Built on Pylon for tenant-scoped state that
            syncs across every client.
          </p>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OrgGate
// ---------------------------------------------------------------------------

function OrgGate({
  currentUser,
  activeOrgId,
  onSelectOrg,
  onSignOut,
  children,
}: {
  currentUser: User;
  activeOrgId: string | null;
  onSelectOrg: (orgId: string | null) => Promise<void>;
  onSignOut: () => void;
  children: (org: Organization) => React.ReactNode;
}) {
  const { data: memberships } = db.useQuery<OrgMember>("OrgMember", {
    where: { userId: currentUser.id },
  });
  const { data: organizations } = db.useQuery<Organization>("Organization");

  const myOrgs = useMemo(() => {
    const byId = new Map<string, Organization>();
    for (const o of organizations ?? []) byId.set(o.id, o);
    const rows: Organization[] = [];
    for (const m of memberships ?? []) {
      const org = byId.get(m.orgId);
      if (org) rows.push(org);
    }
    return rows.sort((a, b) => a.name.localeCompare(b.name));
  }, [memberships, organizations]);

  useEffect(() => {
    if (!activeOrgId && myOrgs.length === 1) {
      void onSelectOrg(myOrgs[0].id);
    }
  }, [activeOrgId, myOrgs]);

  const active = myOrgs.find((o) => o.id === activeOrgId);
  if (active) return <>{children(active)}</>;

  return (
    <OnboardingScreen
      currentUser={currentUser}
      myOrgs={myOrgs}
      onSelectOrg={onSelectOrg}
      onSignOut={onSignOut}
    />
  );
}

function OnboardingScreen({
  currentUser,
  myOrgs,
  onSelectOrg,
  onSignOut,
}: {
  currentUser: User;
  myOrgs: Organization[];
  onSelectOrg: (orgId: string) => Promise<void>;
  onSignOut: () => void;
}) {
  const [createOpen, setCreateOpen] = useState(myOrgs.length === 0);

  return (
    <div
      className="flex min-h-screen items-center justify-center"
      style={{
        background:
          "radial-gradient(circle at 30% 30%, rgba(124, 58, 237, 0.35) 0%, var(--color-background) 70%)",
      }}
    >
      <Card className="w-[min(480px,92vw)] p-7">
        <BrandRow className="mb-5" />
        <h2 className="mb-3 text-xl font-semibold">
          Hi {currentUser.displayName.split(" ")[0]} — pick a workspace
        </h2>
        {myOrgs.length === 0 ? (
          <p className="mb-4 text-sm text-muted-foreground">
            You don&rsquo;t have any workspaces yet. Create your first one.
          </p>
        ) : (
          <div className="mb-4 flex flex-col gap-1.5">
            {myOrgs.map((o) => (
              <button
                key={o.id}
                onClick={() => onSelectOrg(o.id)}
                className="flex flex-col items-start gap-0.5 rounded-md border bg-secondary/40 px-3 py-2.5 text-left text-sm transition-colors hover:bg-accent"
              >
                <span className="font-medium">{o.name}</span>
                <span className="text-xs text-muted-foreground">{o.slug}</span>
              </button>
            ))}
          </div>
        )}
        <div className="flex gap-2">
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" />
            New workspace
          </Button>
          <Button variant="outline" onClick={onSignOut}>
            Sign out
          </Button>
        </div>
      </Card>
      <NewOrgModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={async (orgId) => {
          await onSelectOrg(orgId);
        }}
      />
    </div>
  );
}

function NewOrgModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (orgId: string) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      const autoSlug =
        slug.trim() ||
        name
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/^-|-$/g, "");
      const r = await callFn<{ orgId: string }>("createOrganization", {
        name,
        slug: autoSlug,
      });
      await onCreated(r.orgId);
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create workspace</DialogTitle>
          <DialogDescription>
            Every workspace comes seeded with three starter agents and a sample
            pipeline.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-3">
          <Field label="Workspace name">
            <Input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Acme Labs"
            />
          </Field>
          <Field label="URL slug (optional)">
            <Input
              value={slug}
              onChange={(e) => setSlug(e.target.value)}
              placeholder="acme-labs"
            />
          </Field>
          {err && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {err}
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button disabled={busy || !name.trim()}>
              {busy ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

function Workspace({
  org,
  currentUser,
  page,
  setPage,
  activeRunId,
  setActiveRunId,
  onSignOut,
}: {
  org: Organization;
  currentUser: User;
  page: Page;
  setPage: (p: Page) => void;
  activeRunId: string | null;
  setActiveRunId: (id: string | null) => void;
  onSignOut: () => void;
}) {
  const { data: agents } = db.useQuery<Agent>("Agent");
  const { data: pipelines } = db.useQuery<Pipeline>("Pipeline");
  const { data: runs } = db.useQuery<Run>("Run");

  const [newRunOpen, setNewRunOpen] = useState(false);
  const [newAgentOpen, setNewAgentOpen] = useState(false);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setNewRunOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="grid h-screen grid-cols-[240px_1fr]">
      <aside className="flex flex-col gap-1 border-r bg-card/60 p-3">
        <div className="mb-4 flex items-center gap-2 px-2">
          <BrandMark />
          <span className="truncate text-sm font-semibold">{org.name}</span>
        </div>

        <div className="px-2 pb-1 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          Workspace
        </div>
        <NavItem
          icon={<Sparkles className="size-4" />}
          label="Runs"
          count={(runs ?? []).length}
          active={page === "runs"}
          onClick={() => setPage("runs")}
        />
        <NavItem
          icon={<Bot className="size-4" />}
          label="Agents"
          count={(agents ?? []).length}
          active={page === "agents"}
          onClick={() => setPage("agents")}
        />
        <NavItem
          icon={<Workflow className="size-4" />}
          label="Pipelines"
          count={(pipelines ?? []).length}
          active={page === "pipelines"}
          onClick={() => setPage("pipelines")}
        />

        <div className="flex-1" />

        <button
          onClick={onSignOut}
          className="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-foreground/80 transition-colors hover:bg-accent hover:text-accent-foreground"
        >
          <span
            className="grid size-6 place-items-center rounded-full text-[10px] font-semibold text-white"
            style={{ background: currentUser.avatarColor }}
          >
            {currentUser.displayName.charAt(0).toUpperCase()}
          </span>
          <span className="flex-1 text-left">Sign out</span>
          <LogOut className="size-3.5 text-muted-foreground" />
        </button>
      </aside>

      <div className="flex flex-col overflow-hidden">
        <div className="flex h-14 items-center gap-3 border-b px-6">
          <h1 className="text-base font-semibold">
            {page === "runs" && "Runs"}
            {page === "agents" && "Agents"}
            {page === "pipelines" && "Pipelines"}
          </h1>
          <div className="flex-1" />
          {page === "agents" && (
            <Button variant="outline" size="sm" onClick={() => setNewAgentOpen(true)}>
              <Plus className="size-4" />
              New agent
            </Button>
          )}
          <Button size="sm" onClick={() => setNewRunOpen(true)}>
            <Play className="size-4" />
            Start run
          </Button>
        </div>

        <div className="flex-1 overflow-hidden p-6">
          {page === "runs" && (
            <RunsPage
              runs={runs ?? []}
              agents={agents ?? []}
              pipelines={pipelines ?? []}
              activeRunId={activeRunId}
              setActiveRunId={setActiveRunId}
              onNewRun={() => setNewRunOpen(true)}
            />
          )}
          {page === "agents" && (
            <AgentsPage agents={agents ?? []} onNew={() => setNewAgentOpen(true)} />
          )}
          {page === "pipelines" && (
            <PipelinesPage pipelines={pipelines ?? []} agents={agents ?? []} />
          )}
        </div>
      </div>

      <NewRunModal
        open={newRunOpen}
        agents={agents ?? []}
        pipelines={pipelines ?? []}
        onClose={() => setNewRunOpen(false)}
        onStarted={(runId) => {
          setNewRunOpen(false);
          setPage("runs");
          setActiveRunId(runId);
        }}
      />
      <NewAgentModal open={newAgentOpen} onClose={() => setNewAgentOpen(false)} />
    </div>
  );
}

function NavItem({
  icon,
  label,
  count,
  active,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
        active
          ? "bg-accent text-accent-foreground"
          : "text-foreground/80 hover:bg-accent/50 hover:text-foreground",
      )}
    >
      {icon}
      <span className="flex-1 text-left">{label}</span>
      <span className="font-mono text-xs text-muted-foreground">{count}</span>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Runs page
// ---------------------------------------------------------------------------

function RunsPage({
  runs,
  agents,
  pipelines,
  activeRunId,
  setActiveRunId,
  onNewRun,
}: {
  runs: Run[];
  agents: Agent[];
  pipelines: Pipeline[];
  activeRunId: string | null;
  setActiveRunId: (id: string | null) => void;
  onNewRun: () => void;
}) {
  const sorted = useMemo(
    () => [...runs].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1)),
    [runs],
  );
  const agentById = useMemo(
    () => new Map(agents.map((a) => [a.id, a])),
    [agents],
  );
  const pipelineById = useMemo(
    () => new Map(pipelines.map((p) => [p.id, p])),
    [pipelines],
  );

  if (sorted.length === 0) {
    return (
      <Empty
        title="No runs yet"
        sub="Kick off a run to see live streaming output."
        action={
          <Button onClick={onNewRun}>
            <Play className="size-4" />
            Start run
          </Button>
        }
      />
    );
  }

  return (
    <div className="grid h-full grid-cols-[minmax(380px,1fr)_2fr] gap-5 overflow-hidden">
      <Card className="flex flex-col overflow-hidden">
        <div className="flex-1 overflow-y-auto">
          {sorted.map((run) => {
            const target =
              (run.agentId && agentById.get(run.agentId)?.name) ||
              (run.pipelineId && pipelineById.get(run.pipelineId)?.name) ||
              "—";
            return (
              <button
                key={run.id}
                onClick={() => setActiveRunId(run.id)}
                className={cn(
                  "grid w-full grid-cols-[12px_1fr_auto_auto] items-center gap-3 border-b border-border/50 px-4 py-3 text-left transition-colors last:border-b-0 hover:bg-accent/40",
                  activeRunId === run.id && "bg-accent",
                )}
              >
                <StatusDot status={run.status} />
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium">{run.title}</div>
                  <div className="truncate text-xs text-muted-foreground">
                    {target}
                  </div>
                </div>
                <span className="text-xs text-muted-foreground">
                  {shortTime(run.createdAt)}
                </span>
                <StatusBadge status={run.status} />
              </button>
            );
          })}
        </div>
      </Card>
      <div className="overflow-hidden">
        {activeRunId ? (
          <RunDetail runId={activeRunId} agents={agents} pipelines={pipelines} />
        ) : (
          <Card className="flex h-full items-center justify-center">
            <p className="text-sm text-muted-foreground">
              Select a run to watch it stream.
            </p>
          </Card>
        )}
      </div>
    </div>
  );
}

function RunDetail({
  runId,
  agents,
  pipelines,
}: {
  runId: string;
  agents: Agent[];
  pipelines: Pipeline[];
}) {
  const { data: run } = db.useQueryOne<Run>("Run", runId);
  const { data: steps } = db.useQuery<RunStep>("RunStep", { where: { runId } });
  const { data: messages } = db.useQuery<Message>("Message", { where: { runId } });
  const transcriptRef = useRef<HTMLDivElement>(null);
  const agentById = useMemo(
    () => new Map(agents.map((a) => [a.id, a])),
    [agents],
  );

  useEffect(() => {
    if (!transcriptRef.current) return;
    transcriptRef.current.scrollTop = transcriptRef.current.scrollHeight;
  }, [messages?.length, (messages ?? [])[messages?.length ? messages.length - 1 : 0]?.content?.length]);

  if (!run) {
    return (
      <Card className="flex h-full items-center justify-center text-sm text-muted-foreground">
        Run not found.
      </Card>
    );
  }

  const sortedSteps = [...(steps ?? [])].sort((a, b) => a.stepNumber - b.stepNumber);
  const sortedMessages = [...(messages ?? [])].sort((a, b) =>
    a.createdAt < b.createdAt ? -1 : 1,
  );

  const target =
    (run.agentId && agentById.get(run.agentId)?.name) ||
    (run.pipelineId && pipelines.find((p) => p.id === run.pipelineId)?.name) ||
    "—";

  return (
    <Card className="flex h-full flex-col overflow-hidden">
      <div className="border-b px-6 py-4">
        <div className="flex items-center gap-3">
          <StatusDot status={run.status} />
          <span className="text-base font-semibold">{run.title}</span>
          <StatusBadge status={run.status} />
        </div>
        <div className="mt-1.5 text-xs text-muted-foreground">
          {target} · {sortedSteps.length} step
          {sortedSteps.length === 1 ? "" : "s"} ·{" "}
          {fmtTokens(run.tokensIn, run.tokensOut)}
        </div>
      </div>
      <div ref={transcriptRef} className="flex-1 overflow-y-auto px-6 py-4">
        {sortedSteps.map((step) => {
          const agent = agentById.get(step.agentId);
          const stepMsgs = sortedMessages.filter((m) => m.runStepId === step.id);
          return (
            <div key={step.id} className="mb-4">
              <div className="py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Step {step.stepNumber} · {agent?.name ?? "unknown"} · {step.status}
              </div>
              {stepMsgs.map((m) => (
                <div key={m.id} className="my-2 flex gap-3">
                  <div
                    className={cn(
                      "grid size-8 shrink-0 place-items-center rounded-full text-sm",
                      m.role === "user"
                        ? "bg-primary/15 text-primary"
                        : "bg-muted text-foreground",
                    )}
                  >
                    {m.role === "user" ? "🧑" : agent?.avatarEmoji ?? "🤖"}
                  </div>
                  <div className="flex-1">
                    <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                      {m.role}
                    </div>
                    <div className="whitespace-pre-wrap text-sm leading-relaxed">
                      {m.content}
                      {m.role === "assistant" && step.status === "running" && (
                        <span className="ml-0.5 inline-block h-3.5 w-1 animate-pulse bg-primary align-middle" />
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Agents page
// ---------------------------------------------------------------------------

function AgentsPage({
  agents,
  onNew,
}: {
  agents: Agent[];
  onNew: () => void;
}) {
  if (agents.length === 0) {
    return (
      <Empty
        title="No agents yet"
        sub="Create your first agent to define a reusable persona."
        action={
          <Button onClick={onNew}>
            <Plus className="size-4" />
            New agent
          </Button>
        }
      />
    );
  }
  return (
    <div className="grid auto-rows-fr grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
      {agents.map((a) => (
        <Card key={a.id} className="p-4 transition-colors hover:border-primary/40">
          <div className="text-3xl">{a.avatarEmoji}</div>
          <div className="mt-2 text-base font-semibold">{a.name}</div>
          <div className="mt-0.5 text-xs text-muted-foreground">{a.role}</div>
          <div className="mt-3 inline-flex">
            <Badge variant="secondary" className="font-mono text-[10px]">
              {a.model}
            </Badge>
          </div>
        </Card>
      ))}
    </div>
  );
}

function NewAgentModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [name, setName] = useState("");
  const [role, setRole] = useState("");
  const [systemPrompt, setSystemPrompt] = useState(
    "You are a helpful, concise assistant.",
  );
  const [model, setModel] = useState("claude-sonnet-4-6");
  const [emoji, setEmoji] = useState("🤖");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      await callFn("createAgent", {
        name,
        role,
        systemPrompt,
        model,
        avatarEmoji: emoji,
      });
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>New agent</DialogTitle>
          <DialogDescription>
            Define a persona the whole workspace can re-use.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-3">
          <div className="grid grid-cols-[80px_1fr] gap-3">
            <Field label="Emoji">
              <Input
                value={emoji}
                onChange={(e) => setEmoji(e.target.value)}
                maxLength={4}
              />
            </Field>
            <Field label="Name">
              <Input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="Researcher"
              />
            </Field>
          </div>
          <Field label="Role (short)">
            <Input
              value={role}
              onChange={(e) => setRole(e.target.value)}
              placeholder="Gathers background facts"
            />
          </Field>
          <Field label="System prompt">
            <Textarea
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              rows={5}
            />
          </Field>
          <Field label="Model">
            <Select value={model} onValueChange={setModel}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="claude-opus-4-7">claude-opus-4-7</SelectItem>
                <SelectItem value="claude-sonnet-4-6">claude-sonnet-4-6</SelectItem>
                <SelectItem value="claude-haiku-4-5">claude-haiku-4-5</SelectItem>
              </SelectContent>
            </Select>
          </Field>
          {err && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {err}
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button disabled={busy || !name.trim()}>
              {busy ? "Saving…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Pipelines page
// ---------------------------------------------------------------------------

function PipelinesPage({
  pipelines,
  agents,
}: {
  pipelines: Pipeline[];
  agents: Agent[];
}) {
  if (pipelines.length === 0) {
    return (
      <Empty
        title="No pipelines yet"
        sub='Pipelines chain agents. The default "Brief & Draft" was created with your workspace.'
      />
    );
  }
  return (
    <div className="flex flex-col gap-3">
      {pipelines.map((p) => (
        <PipelineCard key={p.id} pipeline={p} agents={agents} />
      ))}
    </div>
  );
}

function PipelineCard({
  pipeline,
  agents,
}: {
  pipeline: Pipeline;
  agents: Agent[];
}) {
  const { data: steps } = db.useQuery<PipelineStep>("PipelineStep", {
    where: { pipelineId: pipeline.id },
  });
  const sorted = [...(steps ?? [])].sort((a, b) => a.position - b.position);
  const agentById = useMemo(
    () => new Map(agents.map((a) => [a.id, a])),
    [agents],
  );

  return (
    <Card>
      <CardContent className="p-5">
        <div className="flex items-baseline justify-between">
          <div>
            <div className="text-base font-semibold">{pipeline.name}</div>
            {pipeline.description && (
              <div className="mt-1 text-sm text-muted-foreground">
                {pipeline.description}
              </div>
            )}
          </div>
          <span className="text-xs text-muted-foreground">
            {sorted.length} step{sorted.length === 1 ? "" : "s"}
          </span>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-2">
          {sorted.map((s, i) => {
            const agent = agentById.get(s.agentId);
            return (
              <React.Fragment key={s.id}>
                {i > 0 && <ChevronRight className="size-3 text-muted-foreground" />}
                <div className="flex items-center gap-2 rounded-md border bg-secondary/40 px-2.5 py-1 text-sm">
                  <span>{agent?.avatarEmoji}</span>
                  <span className="font-medium">{agent?.name ?? "(deleted)"}</span>
                </div>
              </React.Fragment>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// New run modal
// ---------------------------------------------------------------------------

function NewRunModal({
  open,
  agents,
  pipelines,
  onClose,
  onStarted,
}: {
  open: boolean;
  agents: Agent[];
  pipelines: Pipeline[];
  onClose: () => void;
  onStarted: (runId: string) => void;
}) {
  const [mode, setMode] = useState<"agent" | "pipeline">(
    pipelines.length > 0 ? "pipeline" : "agent",
  );
  const [agentId, setAgentId] = useState(agents[0]?.id ?? "");
  const [pipelineId, setPipelineId] = useState(pipelines[0]?.id ?? "");
  const [input, setInput] = useState("");
  const [title, setTitle] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function resolveSteps(): Array<{
    agentId: string;
    systemPrompt: string;
    instruction: string;
  }> {
    const agentById = new Map(agents.map((a) => [a.id, a]));
    if (mode === "agent") {
      const a = agentById.get(agentId);
      if (!a) return [];
      return [
        { agentId: a.id, systemPrompt: a.systemPrompt, instruction: "{{input}}" },
      ];
    }
    const stepRows = db.sync.store.list("PipelineStep") as PipelineStep[];
    const ours = stepRows
      .filter((s) => s.pipelineId === pipelineId)
      .sort((a, b) => a.position - b.position);
    return ours
      .map((s) => {
        const a = agentById.get(s.agentId);
        if (!a) return null;
        return {
          agentId: a.id,
          systemPrompt: a.systemPrompt,
          instruction: s.instruction,
        };
      })
      .filter(
        (x): x is { agentId: string; systemPrompt: string; instruction: string } =>
          x != null,
      );
  }

  async function submit(e?: React.FormEvent) {
    e?.preventDefault();
    setBusy(true);
    setErr(null);
    const steps = resolveSteps();
    if (steps.length === 0) {
      setErr("Could not resolve steps — pipeline may be missing its agent(s).");
      setBusy(false);
      return;
    }
    const args = {
      agentId: mode === "agent" ? agentId : undefined,
      pipelineId: mode === "pipeline" ? pipelineId : undefined,
      input,
      title: title.trim() || undefined,
      steps,
    };
    const before = new Set<string>(
      db.sync.store.list("Run").map((r) => r.id as string),
    );
    callFn<{ runId: string }>("startRun", args).catch((e) => {
      setErr(e instanceof Error ? e.message : String(e));
      setBusy(false);
    });
    let tries = 0;
    const poll = setInterval(() => {
      tries++;
      const rows = db.sync.store.list("Run") as Run[];
      const fresh = rows.find((r) => !before.has(r.id));
      if (fresh) {
        clearInterval(poll);
        onStarted(fresh.id);
      } else if (tries > 40) {
        clearInterval(poll);
        setErr("run didn't appear — check server logs");
        setBusy(false);
      }
    }, 100);
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Start a run</DialogTitle>
          <DialogDescription>
            Pick an agent or pipeline, give it an input, and watch it stream.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="flex flex-col gap-3">
          <div className="flex gap-2">
            <Button
              type="button"
              variant={mode === "pipeline" ? "default" : "outline"}
              onClick={() => setMode("pipeline")}
              disabled={pipelines.length === 0}
              size="sm"
            >
              <ListTree className="size-4" />
              Pipeline
            </Button>
            <Button
              type="button"
              variant={mode === "agent" ? "default" : "outline"}
              onClick={() => setMode("agent")}
              size="sm"
            >
              <Bot className="size-4" />
              Single agent
            </Button>
          </div>

          {mode === "agent" ? (
            <Field label="Agent">
              <Select value={agentId} onValueChange={setAgentId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {agents.map((a) => (
                    <SelectItem key={a.id} value={a.id}>
                      {a.avatarEmoji} {a.name} — {a.role}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          ) : (
            <Field label="Pipeline">
              <Select value={pipelineId} onValueChange={setPipelineId}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {pipelines.map((p) => (
                    <SelectItem key={p.id} value={p.id}>
                      {p.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
          )}

          <Field label="Input">
            <Textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="What should the agents work on?"
              rows={4}
              autoFocus
            />
          </Field>
          <Field label="Title (optional)">
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Defaults to the first line of your input"
            />
          </Field>

          {err && (
            <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {err}
            </div>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button
              disabled={
                busy ||
                !input.trim() ||
                (mode === "agent" ? !agentId : !pipelineId)
              }
            >
              <Play className="size-4" />
              {busy ? "Running…" : "Run"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Bits
// ---------------------------------------------------------------------------

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

function Empty({
  title,
  sub,
  action,
}: {
  title: string;
  sub: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="grid h-full place-items-center">
      <div className="flex flex-col items-center gap-3 text-center">
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="max-w-sm text-sm text-muted-foreground">{sub}</p>
        {action}
      </div>
    </div>
  );
}

function StatusDot({ status }: { status: string }) {
  const cls =
    status === "completed"
      ? "bg-emerald-400"
      : status === "running"
      ? "animate-pulse bg-primary"
      : status === "failed"
      ? "bg-destructive"
      : "bg-muted-foreground/40";
  return <span className={cn("inline-block size-2.5 rounded-full", cls)} />;
}

function StatusBadge({ status }: { status: string }) {
  const variant =
    status === "completed"
      ? "success"
      : status === "running"
      ? "default"
      : status === "failed"
      ? "destructive"
      : "secondary";
  return (
    <Badge variant={variant as "default"} className="font-mono text-[10px]">
      {status}
    </Badge>
  );
}

function BrandRow({ className }: { className?: string }) {
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <BrandMark />
      <span className="text-base font-semibold">Crew</span>
    </div>
  );
}

function BrandMark() {
  return (
    <div className="grid size-7 place-items-center rounded-md bg-primary text-sm font-bold text-primary-foreground">
      C
    </div>
  );
}

function shortTime(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const diff = (now.getTime() - d.getTime()) / 1000;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return d.toLocaleDateString();
}

function fmtTokens(tokensIn?: number | null, tokensOut?: number | null): string {
  const i = tokensIn ?? 0;
  const o = tokensOut ?? 0;
  if (i === 0 && o === 0) return "—";
  return `${i}↑ ${o}↓ tokens`;
}
