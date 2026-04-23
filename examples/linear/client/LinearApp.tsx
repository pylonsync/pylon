/**
 * Pylon Linear clone — org → teams → issues + cycles + projects +
 * labels + comments. Keyboard-driven: j/k navigate, c create, s/p/a set
 * state/priority/assignee, ⌘K command palette, Esc close drawer.
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "linear" });
configureClient({ baseUrl: BASE_URL, appName: "linear" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type User = { id: string; email: string; displayName: string; avatarColor: string };
type Organization = { id: string; name: string; slug: string };
type OrgMember = { id: string; userId: string; orgId: string; role: string };
type Team = {
  id: string;
  orgId: string;
  name: string;
  key: string;
  description?: string | null;
  issueSequence: number;
};
type Issue = {
  id: string;
  orgId: string;
  teamId: string;
  number: number;
  title: string;
  description?: string | null;
  state: string;
  priority: number;
  assigneeId?: string | null;
  creatorId: string;
  cycleId?: string | null;
  projectId?: string | null;
  estimate?: number | null;
  createdAt: string;
  updatedAt: string;
};
type Comment = {
  id: string;
  orgId: string;
  issueId: string;
  authorId: string;
  body: string;
  createdAt: string;
};
type IssueActivity = {
  id: string;
  orgId: string;
  issueId: string;
  actorId: string;
  kind: string;
  metaJson?: string | null;
  createdAt: string;
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const STATES = [
  { id: "triage", label: "Triage", color: "var(--state-triage)" },
  { id: "backlog", label: "Backlog", color: "var(--state-backlog)" },
  { id: "todo", label: "Todo", color: "var(--state-todo)" },
  { id: "in_progress", label: "In Progress", color: "var(--state-in_progress)" },
  { id: "in_review", label: "In Review", color: "var(--state-in_review)" },
  { id: "done", label: "Done", color: "var(--state-done)" },
  { id: "cancelled", label: "Cancelled", color: "var(--state-cancelled)" },
] as const;
const STATE_BY_ID = Object.fromEntries(STATES.map((s) => [s.id, s]));

const PRIORITIES = [
  { id: 0, label: "No priority" },
  { id: 1, label: "Urgent" },
  { id: 2, label: "High" },
  { id: 3, label: "Medium" },
  { id: 4, label: "Low" },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function initials(name: string | undefined | null): string {
  if (!name) return "·";
  const parts = name.trim().split(/\s+/);
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
function ago(iso: string): string {
  const d = new Date(iso);
  const diff = Date.now() - d.getTime();
  const m = Math.floor(diff / 60000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  const days = Math.floor(h / 24);
  if (days < 7) return `${days}d`;
  return d.toLocaleDateString();
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

type View =
  | { kind: "team"; teamId: string; filter: "active" | "all" | "backlog" | "completed" }
  | { kind: "my" };

export function LinearApp() {
  const [currentUser, setCurrentUser] = useState<User | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as User) : null;
    } catch {
      return null;
    }
  });
  const [activeOrgId, setActiveOrgId] = useState<string | null>(() =>
    localStorage.getItem(storageKey("active_org")),
  );

  useEffect(() => {
    if (currentUser) void db.sync.pull();
  }, [currentUser?.id]);

  // Reconcile server tenant with client activeOrgId.
  useEffect(() => {
    if (!currentUser || !activeOrgId) return;
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    let cancelled = false;
    (async () => {
      try {
        const me = await fetch(`${BASE_URL}/api/auth/me`, {
          headers: { Authorization: `Bearer ${token}` },
        }).then((r) => r.json());
        if (cancelled || me.tenant_id === activeOrgId) return;
        await fetch(`${BASE_URL}/api/auth/select-org`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${token}`,
          },
          body: JSON.stringify({ orgId: activeOrgId }),
        });
        if (!cancelled) await db.sync.pull();
      } catch {}
    })();
    return () => {
      cancelled = true;
    };
  }, [currentUser?.id, activeOrgId]);

  async function signOut() {
    const token = localStorage.getItem(storageKey("token"));
    localStorage.removeItem(storageKey("token"));
    localStorage.removeItem(storageKey("user"));
    localStorage.removeItem(storageKey("active_org"));
    if (token) {
      fetch(`${BASE_URL}/api/auth/session`, {
        method: "DELETE",
        headers: { Authorization: `Bearer ${token}` },
      }).catch(() => {});
    }
    try {
      indexedDB.deleteDatabase("pylon_sync_linear");
    } catch {}
    setCurrentUser(null);
    setActiveOrgId(null);
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
    if (!res.ok) throw new Error(`switch failed (${res.status})`);
    if (orgId) localStorage.setItem(storageKey("active_org"), orgId);
    else localStorage.removeItem(storageKey("active_org"));
    setActiveOrgId(orgId);
    await db.sync.pull();
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
        <Workspace org={org} currentUser={currentUser} onSignOut={signOut} />
      )}
    </OrgGate>
  );
}

function Workspace({
  org,
  currentUser,
  onSignOut,
}: {
  org: Organization;
  currentUser: User;
  onSignOut: () => void;
}) {
  const { data: teams } = db.useQuery<Team>("Team", {
    where: { orgId: org.id },
    orderBy: { name: "asc" },
  });
  const [view, setView] = useState<View>({ kind: "my" });
  const [openIssueId, setOpenIssueId] = useState<string | null>(null);
  const [newIssueOpen, setNewIssueOpen] = useState(false);
  const [newTeamOpen, setNewTeamOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);

  // If nothing's picked yet and we have teams, select the first one.
  useEffect(() => {
    if (view.kind === "my" && teams && teams.length > 0) {
      setView({ kind: "team", teamId: teams[0].id, filter: "active" });
    }
  }, [teams?.length]);

  // Global keyboard shortcuts — just the framework. Specific keys for
  // issue list nav live inside IssueList so they can operate on the
  // focused row.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing =
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable;

      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
        return;
      }
      if (e.key === "Escape") {
        if (paletteOpen) setPaletteOpen(false);
        else if (newIssueOpen) setNewIssueOpen(false);
        else if (newTeamOpen) setNewTeamOpen(false);
        else if (openIssueId) setOpenIssueId(null);
        return;
      }
      if (typing) return;
      if (e.key.toLowerCase() === "c" && !mod) {
        e.preventDefault();
        setNewIssueOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, newIssueOpen, newTeamOpen, openIssueId]);

  return (
    <div className="app">
      <div className="body">
        <Sidebar
          org={org}
          teams={teams ?? []}
          view={view}
          onViewChange={setView}
          onNewTeam={() => setNewTeamOpen(true)}
          currentUser={currentUser}
          onSignOut={onSignOut}
        />
        <IssueList
          org={org}
          teams={teams ?? []}
          view={view}
          currentUser={currentUser}
          onOpen={setOpenIssueId}
          onNewIssue={() => setNewIssueOpen(true)}
        />
      </div>
      {openIssueId && (
        <IssueDrawer
          issueId={openIssueId}
          currentUser={currentUser}
          teams={teams ?? []}
          onClose={() => setOpenIssueId(null)}
        />
      )}
      {newIssueOpen && (
        <NewIssueModal
          teams={teams ?? []}
          currentView={view}
          onClose={() => setNewIssueOpen(false)}
          onCreated={(id) => {
            setNewIssueOpen(false);
            setOpenIssueId(id);
          }}
        />
      )}
      {newTeamOpen && (
        <NewTeamModal
          onClose={() => setNewTeamOpen(false)}
          onCreated={(teamId) => {
            setNewTeamOpen(false);
            setView({ kind: "team", teamId, filter: "active" });
          }}
        />
      )}
      {paletteOpen && (
        <CommandPalette
          teams={teams ?? []}
          onClose={() => setPaletteOpen(false)}
          onOpenIssue={(id) => {
            setPaletteOpen(false);
            setOpenIssueId(id);
          }}
          onGoToTeam={(id) => {
            setPaletteOpen(false);
            setView({ kind: "team", teamId: id, filter: "active" });
          }}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

function Sidebar({
  org,
  teams,
  view,
  onViewChange,
  onNewTeam,
  currentUser,
  onSignOut,
}: {
  org: Organization;
  teams: Team[];
  view: View;
  onViewChange: (v: View) => void;
  onNewTeam: () => void;
  currentUser: User;
  onSignOut: () => void;
}) {
  return (
    <nav className="nav">
      <div className="nav-brand">
        <div className="nav-brand-mark">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
            <path d="M4 7l8-4 8 4v10l-8 4-8-4V7z" stroke="white" strokeWidth="2" strokeLinejoin="round" />
            <path d="M12 11v10M4 7l8 4 8-4" stroke="white" strokeWidth="2" strokeLinejoin="round" />
          </svg>
        </div>
        <div style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {org.name}
        </div>
      </div>
      <div className="nav-section">Your issues</div>
      <div
        className={"nav-item" + (view.kind === "my" ? " active" : "")}
        onClick={() => onViewChange({ kind: "my" })}
      >
        <IconInbox /> My issues
      </div>
      <div
        className="nav-section"
        style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}
      >
        <span>Teams</span>
        <button
          className="btn btn-ghost"
          style={{ padding: "2px 6px", fontSize: 11 }}
          onClick={onNewTeam}
          title="New team"
        >
          +
        </button>
      </div>
      {teams.map((t) => (
        <div key={t.id}>
          <div
            className={
              "nav-item" +
              (view.kind === "team" && view.teamId === t.id && view.filter === "active"
                ? " active"
                : "")
            }
            onClick={() => onViewChange({ kind: "team", teamId: t.id, filter: "active" })}
          >
            <span style={{ fontSize: 10, fontWeight: 600, color: "var(--text-dim)" }}>
              {t.key}
            </span>
            <span style={{ flex: 1 }}>{t.name}</span>
          </div>
          {view.kind === "team" && view.teamId === t.id && (
            <div style={{ paddingLeft: 16 }}>
              {(["active", "backlog", "completed", "all"] as const).map((f) => (
                <div
                  key={f}
                  className={"nav-item" + (view.filter === f ? " active" : "")}
                  onClick={() => onViewChange({ kind: "team", teamId: t.id, filter: f })}
                  style={{ fontSize: 12 }}
                >
                  {f === "active" && "Active"}
                  {f === "backlog" && "Backlog"}
                  {f === "completed" && "Completed"}
                  {f === "all" && "All issues"}
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
      <div style={{ flex: 1 }} />
      <div
        style={{
          padding: "8px 10px",
          display: "flex",
          alignItems: "center",
          gap: 8,
          borderTop: "1px solid var(--border)",
        }}
      >
        <div
          className="avatar avatar-sm"
          style={{ backgroundColor: currentUser.avatarColor }}
        >
          {initials(currentUser.displayName)}
        </div>
        <div style={{ flex: 1, fontSize: 12 }}>{currentUser.displayName}</div>
        <button
          className="btn btn-ghost"
          style={{ padding: "2px 6px", fontSize: 10.5 }}
          onClick={onSignOut}
        >
          Sign out
        </button>
      </div>
    </nav>
  );
}

// ---------------------------------------------------------------------------
// Issue list (keyboard-driven)
// ---------------------------------------------------------------------------

function IssueList({
  org,
  teams,
  view,
  currentUser,
  onOpen,
  onNewIssue,
}: {
  org: Organization;
  teams: Team[];
  view: View;
  currentUser: User;
  onOpen: (id: string) => void;
  onNewIssue: () => void;
}) {
  const { data: allIssues } = db.useQuery<Issue>("Issue", {
    where: { orgId: org.id },
    orderBy: { updatedAt: "desc" },
  });
  const teamById = new Map(teams.map((t) => [t.id, t]));

  const issues = useMemo(() => {
    let list = allIssues ?? [];
    if (view.kind === "my") {
      list = list.filter((i) => i.assigneeId === currentUser.id);
    } else {
      list = list.filter((i) => i.teamId === view.teamId);
      if (view.filter === "active")
        list = list.filter((i) =>
          ["todo", "in_progress", "in_review"].includes(i.state),
        );
      else if (view.filter === "backlog")
        list = list.filter((i) => ["backlog", "triage"].includes(i.state));
      else if (view.filter === "completed")
        list = list.filter((i) => ["done", "cancelled"].includes(i.state));
    }
    // Sort by state group then updated.
    const stateRank: Record<string, number> = {
      triage: 0, backlog: 1, todo: 2, in_progress: 3, in_review: 4, done: 5, cancelled: 6,
    };
    return [...list].sort((a, b) => {
      const sa = stateRank[a.state] ?? 99;
      const sb = stateRank[b.state] ?? 99;
      if (sa !== sb) return sa - sb;
      return b.updatedAt.localeCompare(a.updatedAt);
    });
  }, [allIssues, view, currentUser.id]);

  const [focusIdx, setFocusIdx] = useState(0);
  useEffect(() => setFocusIdx(0), [view]);

  // Keyboard shortcuts for the list — j/k nav, enter open.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const typing =
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable;
      if (typing) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (issues.length === 0) return;
      if (e.key === "j" || e.key === "ArrowDown") {
        e.preventDefault();
        setFocusIdx((i) => Math.min(i + 1, issues.length - 1));
      } else if (e.key === "k" || e.key === "ArrowUp") {
        e.preventDefault();
        setFocusIdx((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const issue = issues[focusIdx];
        if (issue) onOpen(issue.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [issues, focusIdx, onOpen]);

  const heading =
    view.kind === "my"
      ? "My issues"
      : (teamById.get(view.teamId)?.name ?? "Team") +
        " — " +
        view.filter.charAt(0).toUpperCase() +
        view.filter.slice(1);

  return (
    <main className="main">
      <div className="main-header">
        <div className="main-title">{heading}</div>
        <div className="pill pill-gray">{issues.length}</div>
        <div className="spacer" />
        <button className="btn btn-primary" onClick={onNewIssue}>
          <IconPlus /> New issue
        </button>
      </div>
      {issues.length === 0 ? (
        <div className="empty">
          <div className="empty-title">No issues here</div>
          <div className="empty-body">
            Press <kbd className="kbd">C</kbd> to create one.
          </div>
          <button className="btn btn-primary" onClick={onNewIssue}>
            <IconPlus /> New issue
          </button>
        </div>
      ) : (
        <div className="issue-list">
          {issues.map((issue, i) => {
            const team = teamById.get(issue.teamId);
            return (
              <IssueRow
                key={issue.id}
                issue={issue}
                team={team}
                focused={i === focusIdx}
                onClick={() => {
                  setFocusIdx(i);
                  onOpen(issue.id);
                }}
                onHover={() => setFocusIdx(i)}
              />
            );
          })}
        </div>
      )}
      <div className="hint">
        <kbd className="kbd">J</kbd>/<kbd className="kbd">K</kbd> navigate
        · <kbd className="kbd">Enter</kbd> open · <kbd className="kbd">C</kbd>{" "}
        new · <kbd className="kbd">⌘K</kbd> switch
      </div>
    </main>
  );
}

function IssueRow({
  issue,
  team,
  focused,
  onClick,
  onHover,
}: {
  issue: Issue;
  team?: Team;
  focused: boolean;
  onClick: () => void;
  onHover: () => void;
}) {
  const { data: assignee } = db.useQueryOne<User>(
    "User",
    issue.assigneeId ?? "",
  );
  return (
    <div
      className={"issue-row" + (focused ? " focused" : "")}
      onClick={onClick}
      onMouseEnter={onHover}
    >
      <StateIcon state={issue.state} />
      <PriorityIcon priority={issue.priority} />
      <div className="issue-ident">
        {team ? `${team.key}-${issue.number}` : issue.number}
      </div>
      <div className="issue-title">{issue.title}</div>
      <div className="issue-meta">
        {issue.estimate ? <span>{issue.estimate}</span> : null}
        <span>{ago(issue.updatedAt)}</span>
        {assignee ? (
          <div
            className="avatar avatar-xs"
            style={{ backgroundColor: assignee.avatarColor }}
            title={assignee.displayName}
          >
            {initials(assignee.displayName)}
          </div>
        ) : (
          <div
            className="avatar avatar-xs"
            style={{
              backgroundColor: "transparent",
              border: "1.5px dashed var(--text-dim)",
              color: "var(--text-dim)",
            }}
          >
            ·
          </div>
        )}
      </div>
    </div>
  );
}

function StateIcon({ state }: { state: string }) {
  const def = STATE_BY_ID[state];
  const color = def?.color ?? "var(--text-dim)";
  if (state === "done") {
    return (
      <span className="state-icon" title={def.label}>
        <svg width="14" height="14" viewBox="0 0 14 14">
          <circle cx="7" cy="7" r="6" fill={color} />
          <path d="M4 7l2 2 4-4" stroke="white" strokeWidth="2" fill="none" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </span>
    );
  }
  if (state === "cancelled") {
    return (
      <span className="state-icon" title={def.label}>
        <svg width="14" height="14" viewBox="0 0 14 14">
          <circle cx="7" cy="7" r="6" fill={color} />
          <path d="M4.5 4.5l5 5M9.5 4.5l-5 5" stroke="white" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      </span>
    );
  }
  if (state === "in_progress") {
    return (
      <span className="state-icon" title={def.label}>
        <svg width="14" height="14" viewBox="0 0 14 14">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke={color} strokeWidth="1.5" />
          <path d="M7 7 L7 2 A5 5 0 0 1 11.5 9.5 Z" fill={color} />
        </svg>
      </span>
    );
  }
  if (state === "in_review") {
    return (
      <span className="state-icon" title={def.label}>
        <svg width="14" height="14" viewBox="0 0 14 14">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke={color} strokeWidth="1.5" />
          <path d="M7 7 L7 2 A5 5 0 1 1 2 7 Z" fill={color} />
        </svg>
      </span>
    );
  }
  if (state === "todo") {
    return (
      <span className="state-icon" title={def.label}>
        <svg width="14" height="14" viewBox="0 0 14 14">
          <circle cx="7" cy="7" r="5.5" fill="none" stroke={color} strokeWidth="1.5" />
        </svg>
      </span>
    );
  }
  // backlog / triage — dashed circle
  return (
    <span className="state-icon" title={def?.label}>
      <svg width="14" height="14" viewBox="0 0 14 14">
        <circle
          cx="7"
          cy="7"
          r="5.5"
          fill="none"
          stroke={color ?? "var(--text-dim)"}
          strokeWidth="1.5"
          strokeDasharray="2 2"
        />
      </svg>
    </span>
  );
}

function PriorityIcon({ priority }: { priority: number }) {
  if (priority === 0)
    return (
      <span className="priority-icon" title="No priority">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <line
            x1="2"
            y1="6"
            x2="10"
            y2="6"
            stroke="var(--text-dim)"
            strokeWidth="1.5"
            strokeDasharray="2 2"
          />
        </svg>
      </span>
    );
  if (priority === 1)
    return (
      <span className="priority-icon" title="Urgent">
        <svg width="12" height="12" viewBox="0 0 12 12">
          <rect x="1.5" y="1.5" width="9" height="9" rx="1.5" fill="var(--urgent)" />
          <rect x="5.3" y="3" width="1.4" height="4" fill="white" />
          <rect x="5.3" y="8" width="1.4" height="1.4" fill="white" />
        </svg>
      </span>
    );
  const bars = priority === 2 ? 3 : priority === 3 ? 2 : 1;
  return (
    <span className="priority-icon" title={PRIORITIES[priority]?.label}>
      <span className={"priority-bar" + (bars >= 1 ? " active" : "")} />
      <span className={"priority-bar" + (bars >= 2 ? " active" : "")} />
      <span className={"priority-bar" + (bars >= 3 ? " active" : "")} />
    </span>
  );
}

// ---------------------------------------------------------------------------
// Issue drawer
// ---------------------------------------------------------------------------

function IssueDrawer({
  issueId,
  currentUser,
  teams,
  onClose,
}: {
  issueId: string;
  currentUser: User;
  teams: Team[];
  onClose: () => void;
}) {
  const { data: issue } = db.useQueryOne<Issue>("Issue", issueId);
  const { data: comments } = db.useQuery<Comment>("Comment", {
    where: { issueId },
    orderBy: { createdAt: "asc" },
  });
  const { data: activities } = db.useQuery<IssueActivity>("IssueActivity", {
    where: { issueId },
    orderBy: { createdAt: "asc" },
  });
  const team = issue ? teams.find((t) => t.id === issue.teamId) : undefined;

  async function update(patch: Partial<Issue>) {
    try {
      await callFn("updateIssue", { issueId, ...patch });
    } catch (e) {
      alert((e as Error).message);
    }
  }

  if (!issue) return null;

  return (
    <div className="drawer" onClick={onClose}>
      <div className="drawer-panel" onClick={(e) => e.stopPropagation()}>
        <div className="drawer-header">
          <div className="drawer-ident">
            {team ? `${team.key}-${issue.number}` : `#${issue.number}`}
          </div>
          <div className="spacer" />
          <button className="btn btn-ghost" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="drawer-body">
          <div className="drawer-main">
            <div className="issue-heading">{issue.title}</div>
            {issue.description ? (
              <div className="issue-desc">{issue.description}</div>
            ) : (
              <div
                style={{ fontSize: 12.5, color: "var(--text-dim)", fontStyle: "italic" }}
              >
                No description.
              </div>
            )}
            <div
              style={{
                marginTop: 30,
                fontSize: 10.5,
                fontWeight: 600,
                letterSpacing: "0.06em",
                textTransform: "uppercase",
                color: "var(--text-dim)",
                marginBottom: 8,
              }}
            >
              Comments · {(comments ?? []).length}
            </div>
            {(comments ?? []).map((c) => (
              <CommentRow key={c.id} comment={c} />
            ))}
            <CommentComposer issueId={issueId} currentUser={currentUser} />
          </div>
          <div className="drawer-side">
            <div className="drawer-side-label">Status</div>
            <select
              className="select"
              style={{ fontSize: 12, padding: "4px 8px" }}
              value={issue.state}
              onChange={(e) => void update({ state: e.target.value })}
            >
              {STATES.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
            <div className="drawer-side-label">Priority</div>
            <select
              className="select"
              style={{ fontSize: 12, padding: "4px 8px" }}
              value={issue.priority}
              onChange={(e) =>
                void update({ priority: parseInt(e.target.value, 10) })
              }
            >
              {PRIORITIES.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </select>
            <div className="drawer-side-label">Assignee</div>
            <AssigneePicker issue={issue} onChange={(id) => void update({ assigneeId: id })} />
            <div className="drawer-side-label">Estimate</div>
            <input
              className="input"
              type="number"
              min="0"
              step="1"
              style={{ fontSize: 12, padding: "4px 8px" }}
              value={issue.estimate ?? ""}
              onChange={(e) =>
                void update({
                  estimate: e.target.value ? parseFloat(e.target.value) : null,
                })
              }
              placeholder="—"
            />
            <div className="drawer-side-label">Activity</div>
            <div style={{ fontSize: 11.5, color: "var(--text-muted)" }}>
              {(activities ?? []).map((a) => (
                <ActivityRow key={a.id} activity={a} />
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function CommentRow({ comment }: { comment: Comment }) {
  const { data: author } = db.useQueryOne<User>("User", comment.authorId);
  return (
    <div className="comment">
      <div
        className="avatar avatar-sm"
        style={{ backgroundColor: author?.avatarColor || "#c7d2fe" }}
      >
        {initials(author?.displayName)}
      </div>
      <div className="comment-body">
        <div className="comment-meta">
          <strong style={{ color: "var(--text)" }}>
            {author?.displayName ?? "…"}
          </strong>{" "}
          · {ago(comment.createdAt)}
        </div>
        <div className="comment-text">{comment.body}</div>
      </div>
    </div>
  );
}

function CommentComposer({
  issueId,
  currentUser,
}: {
  issueId: string;
  currentUser: User;
}) {
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  async function send() {
    if (!body.trim()) return;
    setBusy(true);
    try {
      await callFn("addComment", { issueId, body });
      setBody("");
    } catch (e) {
      alert((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div style={{ marginTop: 20, display: "flex", gap: 10 }}>
      <div
        className="avatar avatar-sm"
        style={{ backgroundColor: currentUser.avatarColor }}
      >
        {initials(currentUser.displayName)}
      </div>
      <div style={{ flex: 1 }}>
        <textarea
          className="textarea"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder="Leave a comment…"
          rows={2}
        />
        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            marginTop: 6,
          }}
        >
          <button
            className="btn btn-primary"
            onClick={() => void send()}
            disabled={busy || !body.trim()}
          >
            Comment
          </button>
        </div>
      </div>
    </div>
  );
}

function AssigneePicker({
  issue,
  onChange,
}: {
  issue: Issue;
  onChange: (id: string | null) => void;
}) {
  const { data: users } = db.useQuery<User>("User");
  return (
    <select
      className="select"
      style={{ fontSize: 12, padding: "4px 8px" }}
      value={issue.assigneeId ?? ""}
      onChange={(e) => onChange(e.target.value || null)}
    >
      <option value="">Unassigned</option>
      {(users ?? []).map((u) => (
        <option key={u.id} value={u.id}>
          {u.displayName}
        </option>
      ))}
    </select>
  );
}

function ActivityRow({ activity }: { activity: IssueActivity }) {
  const { data: actor } = db.useQueryOne<User>("User", activity.actorId);
  let text = "";
  switch (activity.kind) {
    case "created":
      text = "created the issue";
      break;
    case "state_changed":
      try {
        const m = JSON.parse(activity.metaJson || "{}") as { from?: string; to?: string };
        text = `changed status ${STATE_BY_ID[m.from ?? ""]?.label ?? m.from} → ${STATE_BY_ID[m.to ?? ""]?.label ?? m.to}`;
      } catch {
        text = "changed status";
      }
      break;
    case "priority_changed":
      text = "changed priority";
      break;
    case "assigned":
      text = "changed assignee";
      break;
    case "commented":
      text = "commented";
      break;
    default:
      text = activity.kind;
  }
  return (
    <div style={{ padding: "4px 0", borderBottom: "1px solid var(--border)" }}>
      <strong style={{ color: "var(--text)" }}>
        {actor?.displayName ?? "…"}
      </strong>{" "}
      {text} · {ago(activity.createdAt)}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Modals
// ---------------------------------------------------------------------------

function NewIssueModal({
  teams,
  currentView,
  onClose,
  onCreated,
}: {
  teams: Team[];
  currentView: View;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const defaultTeamId =
    currentView.kind === "team" ? currentView.teamId : teams[0]?.id ?? "";
  const [form, setForm] = useState({
    teamId: defaultTeamId,
    title: "",
    description: "",
    priority: 0,
    state: "todo",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ issueId: string }>("createIssue", {
        teamId: form.teamId,
        title: form.title,
        description: form.description || undefined,
        priority: form.priority,
        state: form.state,
      });
      onCreated(res.issueId);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">New issue</div>
        <div className="modal-subtitle">
          <kbd className="kbd">⌘</kbd>
          <kbd className="kbd">Enter</kbd> to submit
        </div>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Team</span>
            <select
              className="select"
              value={form.teamId}
              onChange={(e) => setForm({ ...form, teamId: e.target.value })}
            >
              {teams.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span className="field-label">Priority</span>
            <select
              className="select"
              value={form.priority}
              onChange={(e) =>
                setForm({ ...form, priority: parseInt(e.target.value, 10) })
              }
            >
              {PRIORITIES.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className="field">
          <span className="field-label">Title</span>
          <input
            autoFocus
            className="input"
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void save();
            }}
            placeholder="What needs to be done?"
          />
        </label>
        <label className="field">
          <span className="field-label">Description</span>
          <textarea
            className="textarea"
            value={form.description}
            onChange={(e) => setForm({ ...form, description: e.target.value })}
            placeholder="Add detail, context, acceptance criteria…"
          />
        </label>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.title.trim() || !form.teamId}
            onClick={() => void save()}
          >
            {busy ? "Creating…" : "Create issue"}
          </button>
        </div>
      </div>
    </div>
  );
}

function NewTeamModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (teamId: string) => void;
}) {
  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const derived = name
      .toUpperCase()
      .replace(/[^A-Z0-9]/g, "")
      .slice(0, 5);
    setKey(derived);
  }, [name]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ teamId: string }>("createTeam", { name, key });
      onCreated(res.teamId);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">New team</div>
        <div className="modal-subtitle">
          Issues get a per-team number prefix like ENG-42.
        </div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
            autoFocus
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Engineering"
          />
        </label>
        <label className="field">
          <span className="field-label">Key (1–10 uppercase)</span>
          <input
            className="input"
            value={key}
            onChange={(e) =>
              setKey(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 10))
            }
          />
        </label>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !name.trim() || !key.trim()}
            onClick={() => void save()}
          >
            {busy ? "Creating…" : "Create team"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

function CommandPalette({
  teams,
  onClose,
  onOpenIssue,
  onGoToTeam,
}: {
  teams: Team[];
  onClose: () => void;
  onOpenIssue: (id: string) => void;
  onGoToTeam: (id: string) => void;
}) {
  const { data: issues } = db.useQuery<Issue>("Issue");
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const teamById = new Map(teams.map((t) => [t.id, t]));
    const out: {
      kind: "issue" | "team";
      id: string;
      label: string;
      meta: string;
    }[] = [];
    // Teams first so they're easy to jump to.
    for (const t of teams) {
      if (!q || t.name.toLowerCase().includes(q) || t.key.toLowerCase().includes(q)) {
        out.push({ kind: "team", id: t.id, label: t.name, meta: t.key });
      }
    }
    for (const i of issues ?? []) {
      const t = teamById.get(i.teamId);
      const ident = t ? `${t.key}-${i.number}` : `#${i.number}`;
      if (
        !q ||
        i.title.toLowerCase().includes(q) ||
        ident.toLowerCase().includes(q)
      ) {
        out.push({ kind: "issue", id: i.id, label: i.title, meta: ident });
      }
      if (out.length > 50) break;
    }
    return out;
  }, [issues, teams, query]);

  useEffect(() => setSel(0), [query]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((s) => Math.min(s + 1, items.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((s) => Math.max(s - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = items[sel];
      if (!item) return;
      if (item.kind === "issue") onOpenIssue(item.id);
      else onGoToTeam(item.id);
    }
  };

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <div className="palette-input-row">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" style={{ color: "var(--text-dim)" }}>
            <circle cx="11" cy="11" r="7" stroke="currentColor" strokeWidth="2" />
            <path d="M21 21l-4.35-4.35" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
          </svg>
          <input
            autoFocus
            className="palette-input"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Jump to issue, team…"
          />
          <kbd className="kbd">Esc</kbd>
        </div>
        <div className="palette-list">
          {items.length === 0 ? (
            <div
              style={{
                padding: "32px 10px",
                textAlign: "center",
                fontSize: 13,
                color: "var(--text-dim)",
              }}
            >
              No matches.
            </div>
          ) : (
            items.map((it, i) => (
              <div
                key={`${it.kind}:${it.id}`}
                className={"palette-item" + (i === sel ? " selected" : "")}
                onClick={() => {
                  if (it.kind === "issue") onOpenIssue(it.id);
                  else onGoToTeam(it.id);
                }}
                onMouseEnter={() => setSel(i)}
              >
                <span
                  style={{
                    fontSize: 11,
                    color: "var(--text-dim)",
                    width: 72,
                    fontVariantNumeric: "tabular-nums",
                  }}
                >
                  {it.meta}
                </span>
                <span style={{ flex: 1 }}>{it.label}</span>
                <span style={{ fontSize: 11, color: "var(--text-dim)" }}>
                  {it.kind === "issue" ? "Issue" : "Team"}
                </span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OrgGate + Login
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
  const { data: orgs } = db.useQuery<Organization>("Organization");
  const myOrgs = useMemo(() => {
    const byId = new Map<string, Organization>();
    for (const o of orgs ?? []) byId.set(o.id, o);
    const out: Organization[] = [];
    for (const m of memberships ?? []) {
      const org = byId.get(m.orgId);
      if (org) out.push(org);
    }
    return out.sort((a, b) => a.name.localeCompare(b.name));
  }, [memberships, orgs]);

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
  const [open, setOpen] = useState(myOrgs.length === 0);
  return (
    <div className="split-screen">
      <div className="auth-panel">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            marginBottom: 20,
          }}
        >
          <div className="nav-brand-mark">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
              <path d="M4 7l8-4 8 4v10l-8 4-8-4V7z" stroke="white" strokeWidth="2" strokeLinejoin="round" />
              <path d="M12 11v10M4 7l8 4 8-4" stroke="white" strokeWidth="2" strokeLinejoin="round" />
            </svg>
          </div>
          <div style={{ fontWeight: 700 }}>Pylon Linear</div>
        </div>
        <div className="auth-title">Hi, {currentUser.displayName}</div>
        <div className="auth-subtitle">
          Pick a workspace or create one.
        </div>
        {myOrgs.map((o) => (
          <button
            key={o.id}
            onClick={() => void onSelectOrg(o.id)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              width: "100%",
              padding: "10px 12px",
              marginBottom: 6,
              background: "var(--surface-hover)",
              border: "1px solid var(--border)",
              borderRadius: 8,
              textAlign: "left",
            }}
          >
            <div
              className="avatar avatar-sm"
              style={{ backgroundColor: "#c7d2fe" }}
            >
              {initials(o.name)}
            </div>
            <div style={{ fontWeight: 500 }}>{o.name}</div>
          </button>
        ))}
        <div style={{ display: "flex", gap: 8, marginTop: 16 }}>
          <button className="btn btn-primary" onClick={() => setOpen(true)}>
            Create workspace
          </button>
          <button className="btn btn-ghost" onClick={onSignOut}>
            Sign out
          </button>
        </div>
      </div>
      {open && (
        <CreateOrgModal
          onClose={() => setOpen(false)}
          onCreated={(orgId) => {
            setOpen(false);
            void onSelectOrg(orgId);
          }}
        />
      )}
    </div>
  );
}

function CreateOrgModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (orgId: string) => void;
}) {
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    setSlug(
      name
        .toLowerCase()
        .trim()
        .replace(/[^a-z0-9\s-]/g, "")
        .replace(/\s+/g, "-")
        .slice(0, 50),
    );
  }, [name]);
  async function save() {
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ orgId: string }>("createOrganization", {
        name,
        slug,
      });
      onCreated(res.orgId);
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Create workspace</div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
            autoFocus
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Acme Eng"
          />
        </label>
        <label className="field">
          <span className="field-label">URL slug</span>
          <input
            className="input"
            value={slug}
            onChange={(e) => setSlug(e.target.value.toLowerCase())}
          />
        </label>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !name.trim() || !slug.trim()}
            onClick={() => void save()}
          >
            {busy ? "Creating…" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [email, setEmail] = useState("eng@acme.example");
  const [name, setName] = useState("Engineer");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go() {
    setLoading(true);
    setErr(null);
    try {
      const session = await fetch(`${BASE_URL}/api/auth/guest`, {
        method: "POST",
      }).then((r) => r.json());
      const token: string = session.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL, appName: "linear" });
      const user = await callFn<User>("upsertUser", {
        email,
        displayName: name,
      });
      await fetch(`${BASE_URL}/api/auth/upgrade`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ user_id: user.id }),
      });
      localStorage.setItem(storageKey("user"), JSON.stringify(user));
      void db.sync.pull();
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
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            marginBottom: 20,
          }}
        >
          <div className="nav-brand-mark">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none">
              <path d="M4 7l8-4 8 4v10l-8 4-8-4V7z" stroke="white" strokeWidth="2" strokeLinejoin="round" />
              <path d="M12 11v10M4 7l8 4 8-4" stroke="white" strokeWidth="2" strokeLinejoin="round" />
            </svg>
          </div>
          <div style={{ fontWeight: 700 }}>Pylon Linear</div>
        </div>
        <div className="auth-title">Sign in</div>
        <div className="auth-subtitle">Track work, ship software.</div>
        <label className="field">
          <span className="field-label">Email</span>
          <input
            autoFocus
            className="input"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </label>
        <label className="field">
          <span className="field-label">Display name</span>
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && go()}
          />
        </label>
        {err && <div className="error-text">{err}</div>}
        <button
          className="btn btn-primary"
          onClick={go}
          disabled={loading}
          style={{ width: "100%", marginTop: 8, padding: "8px 14px" }}
        >
          {loading ? "Signing in…" : "Continue"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

function IconInbox() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path d="M3 8l3-5h12l3 5v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8zM3 8h5l2 3h4l2-3h5" stroke="currentColor" strokeWidth="2" strokeLinejoin="round" />
    </svg>
  );
}
function IconPlus() {
  return (
    <svg width="12" height="12" viewBox="0 0 24 24" fill="none">
      <path d="M12 5v14M5 12h14" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
    </svg>
  );
}
