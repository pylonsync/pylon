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

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "crew" });
configureClient({ baseUrl: BASE_URL, appName: "crew" });

// ---------------------------------------------------------------------------
// Types mirror the entities defined in app.ts
// ---------------------------------------------------------------------------

type User = { id: string; email: string; displayName: string; avatarColor: string };
type Organization = { id: string; name: string; slug: string; createdBy: string; createdAt: string };
type OrgMember = { id: string; userId: string; orgId: string; role: string; joinedAt: string };

type Agent = {
  id: string; orgId: string; name: string; role: string;
  systemPrompt: string; model: string; avatarEmoji: string;
  createdAt: string; createdBy: string;
};

type Pipeline = {
  id: string; orgId: string; name: string;
  description?: string | null; createdBy: string; createdAt: string;
};

type PipelineStep = {
  id: string; orgId: string; pipelineId: string;
  position: number; agentId: string; instruction: string;
};

type Run = {
  id: string; orgId: string;
  pipelineId?: string | null; agentId?: string | null;
  title: string; input: string; status: string;
  startedBy: string; createdAt: string;
  startedAt?: string | null; completedAt?: string | null;
  error?: string | null;
  tokensIn?: number | null; tokensOut?: number | null;
};

type RunStep = {
  id: string; orgId: string; runId: string;
  stepNumber: number; agentId: string;
  input: string; output: string; status: string;
  tokensIn?: number | null; tokensOut?: number | null;
  startedAt?: string | null; completedAt?: string | null;
  error?: string | null;
};

type Message = {
  id: string; orgId: string; runId: string; runStepId: string;
  role: string; content: string; createdAt: string;
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
    try { indexedDB.deleteDatabase("pylon_sync_crew"); } catch {}
    setCurrentUser(null);
    await db.sync.notifySessionChanged();
  }

  async function selectOrg(orgId: string | null) {
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    const res = await fetch(`${BASE_URL}/api/auth/select-org`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
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
      const user = await callFn<User>("upsertUser", {
        email, displayName: name,
      });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
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
    <div className="split-screen">
      <div className="auth-panel">
        <div className="brand" style={{ marginBottom: 24 }}>
          <div className="brand-mark">C</div>
          <div className="brand-name">Crew</div>
        </div>
        <h1 style={{ fontSize: 28, fontWeight: 700, letterSpacing: "-0.01em", marginBottom: 6 }}>
          Sign in
        </h1>
        <p style={{ color: "var(--text-muted)", marginBottom: 24, lineHeight: 1.5 }}>
          Orchestrate AI agents. Chain them into pipelines.
          Watch runs stream live.
        </p>
        <form onSubmit={go}>
          <div className="field">
            <label className="field-label">Email</label>
            <input
              className="field-input"
              placeholder="you@crew.dev"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoFocus
            />
          </div>
          <div className="field">
            <label className="field-label">Display name</label>
            <input
              className="field-input"
              placeholder="Captain"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          {err && (
            <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>
          )}
          <button className="btn-primary" disabled={loading} type="submit" style={{ width: "100%" }}>
            {loading ? "Signing in…" : "Continue"}
          </button>
        </form>
      </div>
      <div className="auth-panel-aside">
        <div className="aside-heading">
          A console for<br />your agent crew.
        </div>
        <div className="aside-sub">
          Define specialized agents. Compose them into pipelines.
          Stream their output in real time. Built on Pylon
          for tenant-scoped state that syncs across every client.
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OrgGate
// ---------------------------------------------------------------------------

function OrgGate({
  currentUser, activeOrgId, onSelectOrg, onSignOut, children,
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
  currentUser, myOrgs, onSelectOrg, onSignOut,
}: {
  currentUser: User;
  myOrgs: Organization[];
  onSelectOrg: (orgId: string) => Promise<void>;
  onSignOut: () => void;
}) {
  const [createOpen, setCreateOpen] = useState(myOrgs.length === 0);

  return (
    <div style={{
      height: "100vh", display: "grid", placeItems: "center",
      background: "radial-gradient(circle at 30% 30%, #7c3aed 0%, #0a0a0f 70%)",
    }}>
      <div style={{ width: "min(480px, 92vw)", background: "var(--surface)",
        border: "1px solid var(--border)", borderRadius: 12, padding: 28 }}>
        <div className="brand" style={{ padding: 0, marginBottom: 20 }}>
          <div className="brand-mark">C</div>
          <div className="brand-name">Crew</div>
        </div>
        <h2 style={{ fontSize: 20, fontWeight: 600, marginBottom: 14 }}>
          Hi {currentUser.displayName.split(" ")[0]} — pick a workspace
        </h2>
        {myOrgs.length === 0 ? (
          <p style={{ color: "var(--text-muted)", marginBottom: 18 }}>
            You don't have any workspaces yet. Create your first one.
          </p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 18 }}>
            {myOrgs.map((o) => (
              <button key={o.id}
                onClick={() => onSelectOrg(o.id)}
                style={{
                  textAlign: "left", padding: "10px 14px", borderRadius: 8,
                  border: "1px solid var(--border)", background: "var(--surface-raised)",
                  color: "var(--text)",
                }}>
                <div style={{ fontWeight: 500 }}>{o.name}</div>
                <div style={{ fontSize: 12, color: "var(--text-dim)" }}>{o.slug}</div>
              </button>
            ))}
          </div>
        )}
        <div style={{ display: "flex", gap: 8 }}>
          <button className="btn-primary" onClick={() => setCreateOpen(true)}>New workspace</button>
          <button className="btn-secondary" onClick={onSignOut}>Sign out</button>
        </div>
      </div>
      {createOpen && (
        <NewOrgModal
          onClose={() => setCreateOpen(false)}
          onCreated={async (orgId) => {
            await onSelectOrg(orgId);
          }}
        />
      )}
    </div>
  );
}

function NewOrgModal({ onClose, onCreated }: {
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
      const autoSlug = slug.trim() || name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
      const r = await callFn<{ orgId: string }>("createOrganization", { name, slug: autoSlug });
      await onCreated(r.orgId);
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <div className="modal-title">Create workspace</div>
        <div className="modal-sub">Every workspace comes seeded with three starter agents and a sample pipeline.</div>
        <div className="field">
          <label className="field-label">Workspace name</label>
          <input className="field-input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Acme Labs" />
        </div>
        <div className="field">
          <label className="field-label">URL slug (optional)</label>
          <input className="field-input" value={slug} onChange={(e) => setSlug(e.target.value)} placeholder="acme-labs" />
        </div>
        {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create"}</button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Workspace
// ---------------------------------------------------------------------------

function Workspace({
  org, currentUser, page, setPage, activeRunId, setActiveRunId, onSignOut,
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
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">C</div>
          <div className="brand-name">{org.name}</div>
        </div>

        <div className="nav-section">Workspace</div>
        <div className={`nav-item ${page === "runs" ? "active" : ""}`} onClick={() => setPage("runs")}>
          <span>Runs</span>
          <span className="nav-item-count">{(runs ?? []).length}</span>
        </div>
        <div className={`nav-item ${page === "agents" ? "active" : ""}`} onClick={() => setPage("agents")}>
          <span>Agents</span>
          <span className="nav-item-count">{(agents ?? []).length}</span>
        </div>
        <div className={`nav-item ${page === "pipelines" ? "active" : ""}`} onClick={() => setPage("pipelines")}>
          <span>Pipelines</span>
          <span className="nav-item-count">{(pipelines ?? []).length}</span>
        </div>

        <div style={{ flex: 1 }} />

        <div className="nav-item" onClick={onSignOut}>
          <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{
              width: 22, height: 22, borderRadius: "50%",
              background: currentUser.avatarColor, color: "white",
              display: "grid", placeItems: "center",
              fontSize: 11, fontWeight: 600,
            }}>
              {currentUser.displayName.charAt(0).toUpperCase()}
            </span>
            Sign out
          </span>
        </div>
      </aside>

      <div className="main">
        <div className="topbar">
          <div className="topbar-title">
            {page === "runs" && "Runs"}
            {page === "agents" && "Agents"}
            {page === "pipelines" && "Pipelines"}
          </div>
          <div className="topbar-spacer" />
          {page === "agents" && (
            <button className="topbar-btn" onClick={() => setNewAgentOpen(true)}>+ New agent</button>
          )}
          {(page === "runs" || page === "agents" || page === "pipelines") && (
            <button className="topbar-btn" onClick={() => setNewRunOpen(true)}>
              ▶ Start run
            </button>
          )}
        </div>

        <div className="page">
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

      {newRunOpen && (
        <NewRunModal
          agents={agents ?? []}
          pipelines={pipelines ?? []}
          onClose={() => setNewRunOpen(false)}
          onStarted={(runId) => {
            setNewRunOpen(false);
            setPage("runs");
            setActiveRunId(runId);
          }}
        />
      )}
      {newAgentOpen && (
        <NewAgentModal onClose={() => setNewAgentOpen(false)} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Runs page
// ---------------------------------------------------------------------------

function RunsPage({
  runs, agents, pipelines, activeRunId, setActiveRunId, onNewRun,
}: {
  runs: Run[]; agents: Agent[]; pipelines: Pipeline[];
  activeRunId: string | null; setActiveRunId: (id: string | null) => void;
  onNewRun: () => void;
}) {
  const sorted = useMemo(
    () => [...runs].sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1)),
    [runs],
  );
  const agentById = useMemo(() => new Map(agents.map((a) => [a.id, a])), [agents]);
  const pipelineById = useMemo(() => new Map(pipelines.map((p) => [p.id, p])), [pipelines]);

  if (sorted.length === 0) {
    return (
      <div className="empty">
        <div className="empty-title">No runs yet</div>
        <div className="empty-sub">Kick off a run to see live streaming output.</div>
        <button className="btn-primary" onClick={onNewRun}>▶ Start run</button>
      </div>
    );
  }

  return (
    <div style={{ display: "grid", gridTemplateColumns: "minmax(420px, 1fr) 2fr", gap: 20, height: "100%" }}>
      <div className="runs-list">
        {sorted.map((run) => {
          const target =
            (run.agentId && agentById.get(run.agentId)?.name) ||
            (run.pipelineId && pipelineById.get(run.pipelineId)?.name) ||
            "—";
          return (
            <div
              key={run.id}
              className={`run-row ${activeRunId === run.id ? "active" : ""}`}
              onClick={() => setActiveRunId(run.id)}
            >
              <span className={`run-dot ${run.status}`} />
              <span className="run-title">{run.title}</span>
              <span className="run-target">{target}</span>
              <span className="run-time">{shortTime(run.createdAt)}</span>
              <span className={`run-status ${run.status}`}>{run.status}</span>
            </div>
          );
        })}
      </div>
      <div>
        {activeRunId ? (
          <RunDetail runId={activeRunId} agents={agents} pipelines={pipelines} />
        ) : (
          <div className="empty" style={{ paddingTop: 120 }}>
            <div className="empty-sub">Select a run to watch it stream.</div>
          </div>
        )}
      </div>
    </div>
  );
}

function RunDetail({ runId, agents, pipelines }: {
  runId: string; agents: Agent[]; pipelines: Pipeline[];
}) {
  const { data: run } = db.useQueryOne<Run>("Run", runId);
  const { data: steps } = db.useQuery<RunStep>("RunStep", { where: { runId } });
  const { data: messages } = db.useQuery<Message>("Message", { where: { runId } });
  const transcriptRef = useRef<HTMLDivElement>(null);
  const agentById = useMemo(() => new Map(agents.map((a) => [a.id, a])), [agents]);

  // Auto-scroll to the bottom when new content streams in.
  useEffect(() => {
    if (!transcriptRef.current) return;
    transcriptRef.current.scrollTop = transcriptRef.current.scrollHeight;
  }, [messages?.length, (messages ?? [])[messages?.length ? messages.length - 1 : 0]?.content?.length]);

  if (!run) return <div className="empty">Run not found.</div>;

  const sortedSteps = [...(steps ?? [])].sort((a, b) => a.stepNumber - b.stepNumber);
  const sortedMessages = [...(messages ?? [])].sort((a, b) =>
    a.createdAt < b.createdAt ? -1 : 1,
  );

  const target =
    (run.agentId && agentById.get(run.agentId)?.name) ||
    (run.pipelineId && pipelines.find((p) => p.id === run.pipelineId)?.name) ||
    "—";

  return (
    <div className="card" style={{ padding: 0, height: "calc(100vh - 120px)", display: "flex", flexDirection: "column" }}>
      <div style={{ padding: "16px 22px", borderBottom: "1px solid var(--border)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span className={`run-dot ${run.status}`} />
          <span style={{ fontSize: 15, fontWeight: 600 }}>{run.title}</span>
          <span className={`run-status ${run.status}`}>{run.status}</span>
        </div>
        <div style={{ color: "var(--text-muted)", fontSize: 12.5, marginTop: 6 }}>
          {target} · {sortedSteps.length} step{sortedSteps.length === 1 ? "" : "s"} ·
          {" "}{fmtTokens(run.tokensIn, run.tokensOut)}
        </div>
      </div>
      <div className="transcript" ref={transcriptRef} style={{ flex: 1, overflowY: "auto" }}>
        {sortedSteps.map((step) => {
          const agent = agentById.get(step.agentId);
          const stepMsgs = sortedMessages.filter((m) => m.runStepId === step.id);
          return (
            <div key={step.id} style={{ marginBottom: 12 }}>
              <div style={{
                fontSize: 11, color: "var(--text-dim)",
                textTransform: "uppercase", letterSpacing: "0.06em",
                fontWeight: 600, padding: "4px 0",
              }}>
                Step {step.stepNumber} · {agent?.name ?? "unknown"} · {step.status}
              </div>
              {stepMsgs.map((m) => (
                <div className="msg" key={m.id}>
                  <div className={`msg-avatar ${m.role === "user" ? "user" : "assistant"}`}>
                    {m.role === "user" ? "You" : agent?.avatarEmoji ?? "🤖"}
                  </div>
                  <div className="msg-body">
                    <div className="msg-role">{m.role}</div>
                    <div className="msg-content">
                      {m.content}
                      {m.role === "assistant" && step.status === "running" && (
                        <span className="cursor-blink" />
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Agents page
// ---------------------------------------------------------------------------

function AgentsPage({ agents, onNew }: { agents: Agent[]; onNew: () => void }) {
  if (agents.length === 0) {
    return (
      <div className="empty">
        <div className="empty-title">No agents yet</div>
        <div className="empty-sub">Create your first agent to define a reusable persona.</div>
        <button className="btn-primary" onClick={onNew}>+ New agent</button>
      </div>
    );
  }
  return (
    <div className="agent-grid">
      {agents.map((a) => (
        <div key={a.id} className="agent-card">
          <div className="agent-emoji">{a.avatarEmoji}</div>
          <div className="agent-name">{a.name}</div>
          <div className="agent-role">{a.role}</div>
          <div className="agent-model">{a.model}</div>
        </div>
      ))}
    </div>
  );
}

function NewAgentModal({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState("");
  const [role, setRole] = useState("");
  const [systemPrompt, setSystemPrompt] = useState("You are a helpful, concise assistant.");
  const [model, setModel] = useState("claude-sonnet-4-6");
  const [emoji, setEmoji] = useState("🤖");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true); setErr(null);
    try {
      await callFn("createAgent", {
        name, role, systemPrompt, model, avatarEmoji: emoji,
      });
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <div className="modal-title">New agent</div>
        <div className="modal-sub">Define a persona the whole workspace can re-use.</div>
        <div style={{ display: "grid", gridTemplateColumns: "80px 1fr", gap: 12 }}>
          <div className="field">
            <label className="field-label">Emoji</label>
            <input className="field-input" value={emoji} onChange={(e) => setEmoji(e.target.value)} maxLength={4} />
          </div>
          <div className="field">
            <label className="field-label">Name</label>
            <input className="field-input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Researcher" />
          </div>
        </div>
        <div className="field">
          <label className="field-label">Role (short)</label>
          <input className="field-input" value={role} onChange={(e) => setRole(e.target.value)} placeholder="Gathers background facts" />
        </div>
        <div className="field">
          <label className="field-label">System prompt</label>
          <textarea className="field-textarea" value={systemPrompt} onChange={(e) => setSystemPrompt(e.target.value)} rows={5} />
        </div>
        <div className="field">
          <label className="field-label">Model</label>
          <select className="field-input" value={model} onChange={(e) => setModel(e.target.value)}>
            <option value="claude-opus-4-7">claude-opus-4-7</option>
            <option value="claude-sonnet-4-6">claude-sonnet-4-6</option>
            <option value="claude-haiku-4-5">claude-haiku-4-5</option>
          </select>
        </div>
        {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary" disabled={busy || !name.trim()}>{busy ? "Saving…" : "Create"}</button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Pipelines page
// ---------------------------------------------------------------------------

function PipelinesPage({ pipelines, agents }: { pipelines: Pipeline[]; agents: Agent[] }) {
  if (pipelines.length === 0) {
    return (
      <div className="empty">
        <div className="empty-title">No pipelines yet</div>
        <div className="empty-sub">Pipelines chain agents. The default "Brief &amp; Draft" was created with your workspace.</div>
      </div>
    );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      {pipelines.map((p) => (
        <PipelineCard key={p.id} pipeline={p} agents={agents} />
      ))}
    </div>
  );
}

function PipelineCard({ pipeline, agents }: { pipeline: Pipeline; agents: Agent[] }) {
  const { data: steps } = db.useQuery<PipelineStep>("PipelineStep", {
    where: { pipelineId: pipeline.id },
  });
  const sorted = [...(steps ?? [])].sort((a, b) => a.position - b.position);
  const agentById = useMemo(() => new Map(agents.map((a) => [a.id, a])), [agents]);

  return (
    <div className="card">
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline" }}>
        <div>
          <div style={{ fontSize: 15, fontWeight: 600 }}>{pipeline.name}</div>
          {pipeline.description && (
            <div style={{ color: "var(--text-muted)", fontSize: 12.5, marginTop: 4 }}>
              {pipeline.description}
            </div>
          )}
        </div>
        <span style={{ color: "var(--text-dim)", fontSize: 12 }}>
          {sorted.length} step{sorted.length === 1 ? "" : "s"}
        </span>
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 14, flexWrap: "wrap" }}>
        {sorted.map((s, i) => {
          const agent = agentById.get(s.agentId);
          return (
            <React.Fragment key={s.id}>
              {i > 0 && <span style={{ color: "var(--text-dim)" }}>→</span>}
              <div style={{
                display: "flex", alignItems: "center", gap: 8,
                padding: "6px 10px", borderRadius: 6,
                background: "var(--surface-raised)",
                border: "1px solid var(--border)",
              }}>
                <span>{agent?.avatarEmoji}</span>
                <span style={{ fontSize: 12.5, fontWeight: 500 }}>{agent?.name ?? "(deleted)"}</span>
              </div>
            </React.Fragment>
          );
        })}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// New run modal
// ---------------------------------------------------------------------------

function NewRunModal({ agents, pipelines, onClose, onStarted }: {
  agents: Agent[]; pipelines: Pipeline[];
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

  // Resolve the flat step list the server expects. The client has the full
  // sync replica and can stitch agent + pipeline-step + instruction, so we
  // hand the server a ready-to-run plan instead of asking it to re-query.
  // (The server can't inherit tenant across nested function calls yet, so
  // a server-side resolver inside the action would otherwise fail.)
  function resolveSteps(): Array<{ agentId: string; systemPrompt: string; instruction: string }> {
    const agentById = new Map(agents.map((a) => [a.id, a]));
    if (mode === "agent") {
      const a = agentById.get(agentId);
      if (!a) return [];
      return [{ agentId: a.id, systemPrompt: a.systemPrompt, instruction: "{{input}}" }];
    }
    const stepRows = db.sync.store.list("PipelineStep") as PipelineStep[];
    const ours = stepRows
      .filter((s) => s.pipelineId === pipelineId)
      .sort((a, b) => a.position - b.position);
    return ours
      .map((s) => {
        const a = agentById.get(s.agentId);
        if (!a) return null;
        return { agentId: a.id, systemPrompt: a.systemPrompt, instruction: s.instruction };
      })
      .filter((x): x is { agentId: string; systemPrompt: string; instruction: string } => x != null);
  }

  async function submit(e?: React.FormEvent) {
    e?.preventDefault();
    setBusy(true); setErr(null);
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
    // Fire-and-forget: the action's first write (runStart) creates the Run
    // and fires a change event, so the Run shows up in the sync store
    // within one WebSocket hop. We poll the store for ~1s to grab the new
    // runId, then open the detail view — by then streaming is already in
    // flight and the transcript panel starts filling itself.
    const before = new Set<string>(db.sync.store.list("Run").map((r) => r.id as string));
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
    <div className="modal-backdrop" onClick={onClose}>
      <form className="modal" onClick={(e) => e.stopPropagation()} onSubmit={submit}>
        <div className="modal-title">Start a run</div>
        <div className="modal-sub">Pick an agent or pipeline, give it an input, and watch it stream.</div>

        <div style={{ display: "flex", gap: 6, marginBottom: 14 }}>
          <button type="button"
            className={mode === "pipeline" ? "btn-primary" : "btn-secondary"}
            onClick={() => setMode("pipeline")} disabled={pipelines.length === 0}>
            Pipeline
          </button>
          <button type="button"
            className={mode === "agent" ? "btn-primary" : "btn-secondary"}
            onClick={() => setMode("agent")}>
            Single agent
          </button>
        </div>

        {mode === "agent" ? (
          <div className="field">
            <label className="field-label">Agent</label>
            <select className="field-input" value={agentId} onChange={(e) => setAgentId(e.target.value)}>
              {agents.map((a) => (
                <option key={a.id} value={a.id}>{a.avatarEmoji} {a.name} — {a.role}</option>
              ))}
            </select>
          </div>
        ) : (
          <div className="field">
            <label className="field-label">Pipeline</label>
            <select className="field-input" value={pipelineId} onChange={(e) => setPipelineId(e.target.value)}>
              {pipelines.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </div>
        )}

        <div className="field">
          <label className="field-label">Input</label>
          <textarea className="field-textarea" value={input} onChange={(e) => setInput(e.target.value)}
            placeholder="What should the agents work on?" rows={4} autoFocus />
        </div>
        <div className="field">
          <label className="field-label">Title (optional)</label>
          <input className="field-input" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Defaults to the first line of your input" />
        </div>

        {err && <div style={{ color: "var(--danger)", fontSize: 12.5, marginBottom: 10 }}>{err}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary"
            disabled={busy || !input.trim() || (mode === "agent" ? !agentId : !pipelineId)}>
            {busy ? "Running…" : "▶ Run"}
          </button>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

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
