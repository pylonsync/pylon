"use client";

/**
 * Pylon Linear clone — org → teams → issues + labels + comments.
 * Keyboard-driven: j/k navigate, c create, ⌘K command palette, Esc close drawer.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import {
  Box,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  CircleDashed,
  CircleDot,
  Inbox,
  Loader2,
  LogOut,
  Plus,
  Search,
  Tag,
  X,
  XCircle,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label as FormLabel } from "@pylonsync/example-ui/label";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { Card } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Avatar, AvatarFallback } from "@pylonsync/example-ui/avatar";
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
import { Sheet, SheetContent } from "@pylonsync/example-ui/sheet";
import { cn } from "@pylonsync/example-ui/utils";

// Same-origin under native SSR: the Pylon binary serves this app and its API
// on one port, so the client talks to its own origin.
const BASE_URL =
  typeof window !== "undefined"
    ? window.location.origin
    : "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "linear" });
configureClient({ baseUrl: BASE_URL, appName: "linear" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type UserRow = {
  id: string;
  email: string;
  displayName: string;
  avatarColor: string;
};
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
type LabelRow = {
  id: string;
  orgId: string;
  teamId: string;
  name: string;
  color: string;
};
type IssueLabel = {
  id: string;
  orgId: string;
  issueId: string;
  labelId: string;
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
  { id: "triage", label: "Triage" },
  { id: "backlog", label: "Backlog" },
  { id: "todo", label: "Todo" },
  { id: "in_progress", label: "In Progress" },
  { id: "in_review", label: "In Review" },
  { id: "done", label: "Done" },
  { id: "cancelled", label: "Cancelled" },
] as const;
const STATE_IDS = STATES.map((s) => s.id);
type StateId = (typeof STATE_IDS)[number];
const STATE_LABELS: Record<string, string> = Object.fromEntries(
  STATES.map((s) => [s.id, s.label]),
);

const PRIORITIES = [
  { id: 0, label: "No priority" },
  { id: 1, label: "Urgent" },
  { id: 2, label: "High" },
  { id: 3, label: "Medium" },
  { id: 4, label: "Low" },
];

// Pre-set label palette for the creation UI.
const LABEL_COLORS = [
  "#ef4444",
  "#f97316",
  "#eab308",
  "#22c55e",
  "#06b6d4",
  "#3b82f6",
  "#8b5cf6",
  "#ec4899",
  "#6b7280",
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
  const [currentUser, setCurrentUser] = useState<UserRow | null>(() => {
    try {
      const token = localStorage.getItem(storageKey("token"));
      const cached = localStorage.getItem(storageKey("user"));
      return token && cached ? (JSON.parse(cached) as UserRow) : null;
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

  useEffect(() => {
    if (!currentUser || !activeOrgId) return;
    const token = localStorage.getItem(storageKey("token"));
    if (!token) return;
    let cancelled = false;
    (async () => {
      try {
        const me = (await fetch(`${BASE_URL}/api/auth/me`, {
          headers: { Authorization: `Bearer ${token}` },
        }).then((r) => r.json())) as { tenant_id?: string };
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
      } catch (err) {
        console.warn("select-org sync failed:", err);
      }
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
    } catch {
      // ignore
    }
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
  currentUser: UserRow;
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

  // Seed demo data into a fresh workspace so the board isn't blank.
  // seedLinear is idempotent server-side (gates on issue count).
  const seedTried = useRef(false);
  useEffect(() => {
    if (seedTried.current || teams === undefined || teams.length === 0) return;
    if (teams.some((t) => (t.issueSequence ?? 0) > 0)) return;
    seedTried.current = true;
    void callFn("seedLinear", {}).catch(() => {});
  }, [teams]);

  useEffect(() => {
    if (view.kind === "my" && teams && teams.length > 0) {
      setView({ kind: "team", teamId: teams[0].id, filter: "active" });
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [teams?.length]);

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
      if (typing) return;
      if (e.key.toLowerCase() === "c" && !mod) {
        e.preventDefault();
        setNewIssueOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="grid h-screen grid-cols-[240px_1fr]">
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

      <Sheet
        open={!!openIssueId}
        onOpenChange={(o) => !o && setOpenIssueId(null)}
      >
        <SheetContent className="w-full max-w-3xl sm:max-w-3xl">
          {openIssueId && (
            <IssueDrawer
              issueId={openIssueId}
              currentUser={currentUser}
              teams={teams ?? []}
              onClose={() => setOpenIssueId(null)}
            />
          )}
        </SheetContent>
      </Sheet>

      <NewIssueModal
        open={newIssueOpen}
        teams={teams ?? []}
        currentView={view}
        onClose={() => setNewIssueOpen(false)}
        onCreated={(id) => {
          setNewIssueOpen(false);
          setOpenIssueId(id);
        }}
      />

      <NewTeamModal
        open={newTeamOpen}
        onClose={() => setNewTeamOpen(false)}
        onCreated={(teamId) => {
          setNewTeamOpen(false);
          setView({ kind: "team", teamId, filter: "active" });
        }}
      />

      <CommandPalette
        open={paletteOpen}
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
  currentUser: UserRow;
  onSignOut: () => void;
}) {
  return (
    <nav className="flex flex-col border-r bg-card/40">
      <div className="flex items-center gap-2 border-b px-3 py-3">
        <BrandMark />
        <span className="truncate text-sm font-semibold">{org.name}</span>
      </div>

      <div className="flex flex-col gap-0.5 p-2">
        <SidebarSection>Your issues</SidebarSection>
        <SidebarItem
          icon={<Inbox className="size-4" />}
          label="My issues"
          active={view.kind === "my"}
          onClick={() => onViewChange({ kind: "my" })}
        />

        <div className="mt-3 flex items-center justify-between px-2 py-1">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Teams
          </span>
          <button
            onClick={onNewTeam}
            className="text-muted-foreground hover:text-foreground"
            title="New team"
          >
            <Plus className="size-3.5" />
          </button>
        </div>

        {teams.map((t) => {
          const isOpen = view.kind === "team" && view.teamId === t.id;
          return (
            <div key={t.id} className="flex flex-col">
              <button
                onClick={() =>
                  onViewChange({ kind: "team", teamId: t.id, filter: "active" })
                }
                className={cn(
                  "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
                  isOpen
                    ? "bg-accent text-accent-foreground"
                    : "text-foreground/80 hover:bg-accent/50",
                )}
              >
                {isOpen ? (
                  <ChevronDown className="size-3 text-muted-foreground" />
                ) : (
                  <ChevronRight className="size-3 text-muted-foreground" />
                )}
                <span className="font-mono text-[10px] font-semibold text-muted-foreground">
                  {t.key}
                </span>
                <span className="truncate">{t.name}</span>
              </button>
              {isOpen && (
                <div className="ml-5 flex flex-col gap-0.5">
                  {(["active", "backlog", "completed", "all"] as const).map((f) => (
                    <button
                      key={f}
                      onClick={() =>
                        onViewChange({
                          kind: "team",
                          teamId: t.id,
                          filter: f,
                        })
                      }
                      className={cn(
                        "rounded-md px-2 py-1 text-left text-xs transition-colors",
                        view.kind === "team" && view.filter === f
                          ? "bg-primary/10 text-primary"
                          : "text-muted-foreground hover:bg-accent/40 hover:text-foreground",
                      )}
                    >
                      {f === "active"
                        ? "Active"
                        : f === "backlog"
                        ? "Backlog"
                        : f === "completed"
                        ? "Completed"
                        : "All issues"}
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>

      <div className="flex-1" />
      <div className="flex items-center gap-2 border-t p-3">
        <Avatar className="size-7" style={{ backgroundColor: currentUser.avatarColor }}>
          <AvatarFallback className="bg-transparent text-[11px] text-white">
            {initials(currentUser.displayName)}
          </AvatarFallback>
        </Avatar>
        <span className="flex-1 truncate text-xs">{currentUser.displayName}</span>
        <Button
          variant="ghost"
          size="icon"
          onClick={onSignOut}
          className="size-7 text-muted-foreground"
          title="Sign out"
        >
          <LogOut className="size-3.5" />
        </Button>
      </div>
    </nav>
  );
}

function SidebarSection({ children }: { children: React.ReactNode }) {
  return (
    <span className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
      {children}
    </span>
  );
}

function SidebarItem({
  icon,
  label,
  active,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
        active
          ? "bg-accent text-accent-foreground"
          : "text-foreground/80 hover:bg-accent/50",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Issue list
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
  currentUser: UserRow;
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
          ["todo", "in_progress", "in_review", "triage"].includes(i.state),
        );
      else if (view.filter === "backlog")
        list = list.filter((i) => i.state === "backlog");
      else if (view.filter === "completed")
        list = list.filter((i) => ["done", "cancelled"].includes(i.state));
    }
    const stateRank: Record<string, number> = {
      triage: 0,
      backlog: 1,
      todo: 2,
      in_progress: 3,
      in_review: 4,
      done: 5,
      cancelled: 6,
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
    <main className="flex flex-col overflow-hidden">
      <header className="flex h-12 items-center gap-3 border-b px-5">
        <h1 className="text-sm font-semibold">{heading}</h1>
        <Badge variant="secondary" className="font-mono text-[10px]">
          {issues.length}
        </Badge>
        <div className="flex-1" />
        <Button size="sm" onClick={onNewIssue}>
          <Plus className="size-4" />
          New issue
        </Button>
      </header>

      {issues.length === 0 ? (
        <div className="grid flex-1 place-items-center">
          <div className="flex flex-col items-center gap-3 text-center">
            <Inbox className="size-10 text-muted-foreground" />
            <h2 className="text-base font-semibold">No issues here</h2>
            <p className="text-sm text-muted-foreground">
              Press <Kbd>C</Kbd> to create one.
            </p>
            <Button onClick={onNewIssue}>
              <Plus className="size-4" />
              New issue
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto">
          {issues.map((issue, i) => (
            <IssueRow
              key={issue.id}
              issue={issue}
              team={teamById.get(issue.teamId)}
              focused={i === focusIdx}
              onClick={() => {
                setFocusIdx(i);
                onOpen(issue.id);
              }}
              onHover={() => setFocusIdx(i)}
            />
          ))}
        </div>
      )}

      <footer className="border-t px-5 py-2 text-[11px] text-muted-foreground">
        <Kbd>J</Kbd>/<Kbd>K</Kbd> navigate · <Kbd>Enter</Kbd> open ·{" "}
        <Kbd>C</Kbd> new · <Kbd>⌘K</Kbd> palette
      </footer>
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
  const { data: assignee } = db.useQueryOne<UserRow>(
    "User",
    issue.assigneeId ?? "",
  );
  return (
    <div
      onClick={onClick}
      onMouseEnter={onHover}
      className={cn(
        "flex cursor-pointer items-center gap-3 border-b border-border/40 px-5 py-2.5 text-sm transition-colors",
        focused ? "bg-accent" : "hover:bg-muted/30",
      )}
    >
      <StateIcon state={issue.state} />
      <PriorityBadge priority={issue.priority} />
      <span className="w-20 shrink-0 font-mono text-xs text-muted-foreground">
        {team ? `${team.key}-${issue.number}` : issue.number}
      </span>
      <span className="flex-1 truncate">{issue.title}</span>
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        {issue.estimate ? (
          <span className="tabular-nums">{issue.estimate}</span>
        ) : null}
        <span>{ago(issue.updatedAt)}</span>
        {assignee ? (
          <Avatar
            className="size-5"
            style={{ backgroundColor: assignee.avatarColor }}
            title={assignee.displayName}
          >
            <AvatarFallback className="bg-transparent text-[9px] text-white">
              {initials(assignee.displayName)}
            </AvatarFallback>
          </Avatar>
        ) : (
          <div className="grid size-5 place-items-center rounded-full border border-dashed text-[9px] text-muted-foreground">
            ·
          </div>
        )}
      </div>
    </div>
  );
}

function StateIcon({ state }: { state: string }) {
  // Use semantic CSS custom properties where possible. For states that
  // need a distinct color we compose a Tailwind class from the design system
  // rather than hard-coding hex values.
  if (state === "done") {
    return <CheckCircle2 className="size-3.5 text-[color:var(--color-state-done,#10b981)]" />;
  }
  if (state === "cancelled") {
    return <XCircle className="size-3.5 text-muted-foreground" />;
  }
  if (state === "in_progress" || state === "in_review") {
    return (
      <CircleDot
        className={cn(
          "size-3.5",
          state === "in_review" ? "text-[color:#a855f7]" : "text-[color:#eab308]",
        )}
      />
    );
  }
  if (state === "todo") {
    return <Circle className="size-3.5 text-foreground/40" />;
  }
  // triage, backlog
  return <CircleDashed className="size-3.5 text-muted-foreground" />;
}

function PriorityBadge({ priority }: { priority: number }) {
  if (priority === 0) {
    return <span className="size-3.5" />;
  }
  if (priority === 1) {
    return (
      <div
        className="grid size-3.5 place-items-center rounded-sm bg-destructive font-mono text-[8px] font-bold text-destructive-foreground"
        title="Urgent"
      >
        !
      </div>
    );
  }
  const bars = priority === 2 ? 3 : priority === 3 ? 2 : 1;
  return (
    <div className="flex items-end gap-px" title={PRIORITIES[priority]?.label}>
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className={cn(
            "w-1 rounded-sm",
            i < bars ? "bg-foreground/70" : "bg-muted",
            i === 0 ? "h-1.5" : i === 1 ? "h-2.5" : "h-3.5",
          )}
        />
      ))}
    </div>
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
  currentUser: UserRow;
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
  const { data: issueLabels } = db.useQuery<IssueLabel>("IssueLabel", {
    where: { issueId },
  });
  const team = issue ? teams.find((t) => t.id === issue.teamId) : undefined;

  // Inline title editing
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const titleRef = useRef<HTMLInputElement>(null);

  // Inline description editing
  const [editingDesc, setEditingDesc] = useState(false);
  const [descDraft, setDescDraft] = useState("");

  // Estimate is committed on blur to avoid calling the server on each keystroke.
  const [estimateDraft, setEstimateDraft] = useState<string>("");
  useEffect(() => {
    if (issue) setEstimateDraft(issue.estimate != null ? String(issue.estimate) : "");
  }, [issue?.estimate]);

  useEffect(() => {
    if (editingTitle && titleRef.current) titleRef.current.focus();
  }, [editingTitle]);

  const update = useCallback(
    async (patch: Record<string, unknown>) => {
      try {
        await callFn("updateIssue", { issueId, ...patch });
      } catch (e) {
        alert((e as Error).message);
      }
    },
    [issueId],
  );

  async function commitTitle() {
    const t = titleDraft.trim();
    if (t && t !== issue?.title) await update({ title: t });
    setEditingTitle(false);
  }

  async function commitDesc() {
    const d = descDraft.trim();
    if (d !== (issue?.description ?? "").trim()) await update({ description: d || null });
    setEditingDesc(false);
  }

  async function commitEstimate() {
    const val = estimateDraft === "" ? null : parseFloat(estimateDraft);
    if (val !== issue?.estimate) await update({ estimate: val });
  }

  if (!issue) return null;

  return (
    <div className="flex h-full flex-col">
      <header className="flex h-12 items-center gap-3 border-b px-5">
        <span className="font-mono text-xs text-muted-foreground">
          {team ? `${team.key}-${issue.number}` : `#${issue.number}`}
        </span>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          onClick={onClose}
          className="size-8"
          title="Close"
        >
          <X className="size-4" />
        </Button>
      </header>
      <div className="grid flex-1 grid-cols-[1fr_220px] overflow-hidden">
        <div className="overflow-y-auto p-6">
          {/* Title — click to edit inline */}
          {editingTitle ? (
            <Input
              ref={titleRef}
              value={titleDraft}
              onChange={(e) => setTitleDraft(e.target.value)}
              onBlur={() => void commitTitle()}
              onKeyDown={(e) => {
                if (e.key === "Enter") void commitTitle();
                if (e.key === "Escape") setEditingTitle(false);
              }}
              className="h-auto border-0 p-0 text-2xl font-semibold leading-tight tracking-tight ring-0 shadow-none focus-visible:ring-1"
            />
          ) : (
            <h2
              className="cursor-text text-2xl font-semibold leading-tight tracking-tight hover:opacity-70"
              title="Click to edit"
              onClick={() => {
                setTitleDraft(issue.title);
                setEditingTitle(true);
              }}
            >
              {issue.title}
            </h2>
          )}

          {/* Description — click to edit inline */}
          <div className="mt-4">
            {editingDesc ? (
              <div className="flex flex-col gap-2">
                <Textarea
                  autoFocus
                  value={descDraft}
                  onChange={(e) => setDescDraft(e.target.value)}
                  rows={6}
                  placeholder="Describe this issue…"
                />
                <div className="flex gap-2">
                  <Button size="sm" onClick={() => void commitDesc()}>
                    Save
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setEditingDesc(false)}
                  >
                    Cancel
                  </Button>
                </div>
              </div>
            ) : (
              <div
                className="cursor-text rounded-md p-2 -mx-2 hover:bg-muted/40 transition-colors"
                title="Click to edit"
                onClick={() => {
                  setDescDraft(issue.description ?? "");
                  setEditingDesc(true);
                }}
              >
                {issue.description ? (
                  <p className="whitespace-pre-wrap text-sm leading-relaxed">
                    {issue.description}
                  </p>
                ) : (
                  <p className="text-sm italic text-muted-foreground">
                    No description. Click to add one.
                  </p>
                )}
              </div>
            )}
          </div>

          {/* Labels panel */}
          {team && (
            <LabelPanel
              issueId={issueId}
              teamId={team.id}
              issueLabels={issueLabels ?? []}
            />
          )}

          <div className="mt-8 mb-3 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Comments · {(comments ?? []).length}
          </div>
          {(comments ?? []).map((c) => (
            <CommentRow key={c.id} comment={c} />
          ))}
          <CommentComposer issueId={issueId} currentUser={currentUser} />
        </div>

        <aside className="flex flex-col gap-3 overflow-y-auto border-l bg-card/40 p-5">
          <SideField label="Status">
            <Select
              value={issue.state}
              onValueChange={(v) => void update({ state: v })}
            >
              <SelectTrigger className="h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATES.map((s) => (
                  <SelectItem key={s.id} value={s.id}>
                    <div className="flex items-center gap-2">
                      <StateIcon state={s.id} />
                      {s.label}
                    </div>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SideField>

          <SideField label="Priority">
            <Select
              value={String(issue.priority)}
              onValueChange={(v) => void update({ priority: parseInt(v, 10) })}
            >
              <SelectTrigger className="h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PRIORITIES.map((p) => (
                  <SelectItem key={p.id} value={String(p.id)}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SideField>

          <SideField label="Assignee">
            <AssigneePicker
              issue={issue}
              onChange={(id) => void update({ assigneeId: id })}
            />
          </SideField>

          <SideField label="Estimate">
            <Input
              type="number"
              min="0"
              step="1"
              className="h-8 text-xs"
              value={estimateDraft}
              onChange={(e) => setEstimateDraft(e.target.value)}
              onBlur={() => void commitEstimate()}
              placeholder="—"
            />
          </SideField>

          <div className="mt-4 mb-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Activity
          </div>
          <div className="flex flex-col gap-1.5 text-[11px] text-muted-foreground">
            {(activities ?? []).map((a) => (
              <ActivityRow key={a.id} activity={a} />
            ))}
          </div>
        </aside>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Label panel (inside issue drawer)
// ---------------------------------------------------------------------------

function LabelPanel({
  issueId,
  teamId,
  issueLabels,
}: {
  issueId: string;
  teamId: string;
  issueLabels: IssueLabel[];
}) {
  const { data: teamLabels } = db.useQuery<LabelRow>("Label", {
    where: { teamId },
    orderBy: { name: "asc" },
  });
  const [showCreate, setShowCreate] = useState(false);
  const [busy, setBusy] = useState<string | null>(null); // labelId being toggled

  const attachedIds = new Set(issueLabels.map((il) => il.labelId));
  const labels = teamLabels ?? [];

  async function toggleLabel(labelId: string) {
    setBusy(labelId);
    try {
      await callFn("toggleIssueLabel", { issueId, labelId });
    } catch (e) {
      alert((e as Error).message);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="mt-6">
      <div className="mb-2 flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        <span className="flex items-center gap-1">
          <Tag className="size-3" />
          Labels
        </span>
        <button
          onClick={() => setShowCreate(true)}
          className="flex items-center gap-0.5 text-muted-foreground hover:text-foreground transition-colors"
          title="Create label"
        >
          <Plus className="size-3" />
        </button>
      </div>

      {labels.length === 0 && !showCreate ? (
        <p className="text-xs text-muted-foreground italic">
          No labels yet.{" "}
          <button
            className="underline hover:text-foreground"
            onClick={() => setShowCreate(true)}
          >
            Create one
          </button>
        </p>
      ) : (
        <div className="flex flex-wrap gap-1.5">
          {labels.map((l) => {
            const attached = attachedIds.has(l.id);
            return (
              <button
                key={l.id}
                onClick={() => void toggleLabel(l.id)}
                disabled={busy === l.id}
                title={attached ? `Remove "${l.name}"` : `Add "${l.name}"`}
                className={cn(
                  "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs transition-all",
                  attached
                    ? "border-transparent text-white opacity-90 hover:opacity-70"
                    : "border-border bg-background text-muted-foreground hover:border-foreground/30 hover:text-foreground",
                )}
                style={attached ? { backgroundColor: l.color } : {}}
              >
                {!attached && (
                  <span
                    className="size-2 rounded-full"
                    style={{ backgroundColor: l.color }}
                  />
                )}
                {l.name}
                {busy === l.id && <Loader2 className="size-3 animate-spin" />}
              </button>
            );
          })}
        </div>
      )}

      {showCreate && (
        <CreateLabelInline
          teamId={teamId}
          onClose={() => setShowCreate(false)}
        />
      )}
    </div>
  );
}

function CreateLabelInline({
  teamId,
  onClose,
}: {
  teamId: string;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [color, setColor] = useState(LABEL_COLORS[0]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function save() {
    if (!name.trim()) return;
    setBusy(true);
    setErr(null);
    try {
      await callFn("createLabel", { teamId, name: name.trim(), color });
      onClose();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 rounded-md border bg-card p-3">
      <div className="mb-2 flex items-center gap-2">
        {LABEL_COLORS.map((c) => (
          <button
            key={c}
            onClick={() => setColor(c)}
            className={cn(
              "size-5 rounded-full transition-transform hover:scale-110",
              color === c && "ring-2 ring-offset-1 ring-foreground",
            )}
            style={{ backgroundColor: c }}
            title={c}
          />
        ))}
      </div>
      <div className="flex gap-2">
        <Input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void save();
            if (e.key === "Escape") onClose();
          }}
          placeholder="Label name"
          className="h-7 text-xs"
        />
        <Button size="sm" onClick={() => void save()} disabled={busy || !name.trim()} className="h-7 px-2">
          {busy ? <Loader2 className="size-3 animate-spin" /> : "Add"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onClose} className="h-7 px-2">
          <X className="size-3" />
        </Button>
      </div>
      {err && <p className="mt-1 text-xs text-destructive">{err}</p>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Side field (right column in issue drawer)
// ---------------------------------------------------------------------------

function SideField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <FormLabel className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {label}
      </FormLabel>
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

function CommentRow({ comment }: { comment: Comment }) {
  const { data: author } = db.useQueryOne<UserRow>("User", comment.authorId);
  return (
    <div className="my-3 flex gap-3">
      <Avatar
        className="size-7"
        style={{ backgroundColor: author?.avatarColor || "#c7d2fe" }}
      >
        <AvatarFallback className="bg-transparent text-[10px] text-white">
          {initials(author?.displayName)}
        </AvatarFallback>
      </Avatar>
      <div className="flex-1">
        <div className="text-xs text-muted-foreground">
          <strong className="text-foreground">
            {author?.displayName ?? "…"}
          </strong>{" "}
          · {ago(comment.createdAt)}
        </div>
        <div className="mt-1 whitespace-pre-wrap text-sm">{comment.body}</div>
      </div>
    </div>
  );
}

function CommentComposer({
  issueId,
  currentUser,
}: {
  issueId: string;
  currentUser: UserRow;
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
    <div className="mt-5 flex gap-3">
      <Avatar
        className="size-7"
        style={{ backgroundColor: currentUser.avatarColor }}
      >
        <AvatarFallback className="bg-transparent text-[10px] text-white">
          {initials(currentUser.displayName)}
        </AvatarFallback>
      </Avatar>
      <div className="flex-1">
        <Textarea
          value={body}
          onChange={(e) => setBody(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void send();
          }}
          placeholder="Leave a comment… (⌘Enter to submit)"
          rows={2}
        />
        <div className="mt-2 flex justify-end">
          <Button
            size="sm"
            onClick={() => void send()}
            disabled={busy || !body.trim()}
          >
            {busy && <Loader2 className="size-4 animate-spin" />}
            Comment
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Assignee picker
// ---------------------------------------------------------------------------

function AssigneePicker({
  issue,
  onChange,
}: {
  issue: Issue;
  onChange: (id: string | null) => void;
}) {
  // Load all users — policy ensures only users visible to this tenant appear.
  const { data: users } = db.useQuery<UserRow>("User");
  return (
    <Select
      value={issue.assigneeId ?? "__none__"}
      onValueChange={(v) => onChange(v === "__none__" ? null : v)}
    >
      <SelectTrigger className="h-8 text-xs">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="__none__">Unassigned</SelectItem>
        {(users ?? []).map((u) => (
          <SelectItem key={u.id} value={u.id}>
            {u.displayName}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

// ---------------------------------------------------------------------------
// Activity row
// ---------------------------------------------------------------------------

function ActivityRow({ activity }: { activity: IssueActivity }) {
  const { data: actor } = db.useQueryOne<UserRow>("User", activity.actorId);
  let text = "";
  switch (activity.kind) {
    case "created":
      text = "created the issue";
      break;
    case "state_changed": {
      try {
        const m = JSON.parse(activity.metaJson || "{}") as {
          from?: string;
          to?: string;
        };
        const fromLabel = STATE_LABELS[m.from ?? ""] ?? m.from ?? "?";
        const toLabel = STATE_LABELS[m.to ?? ""] ?? m.to ?? "?";
        text = `changed status ${fromLabel} → ${toLabel}`;
      } catch {
        text = "changed status";
      }
      break;
    }
    case "priority_changed":
      text = "changed priority";
      break;
    case "assigned":
      text = "changed assignee";
      break;
    case "commented":
      text = "commented";
      break;
    case "renamed":
      text = "renamed the issue";
      break;
    case "estimated":
      text = "changed estimate";
      break;
    default:
      text = activity.kind;
  }
  return (
    <div className="border-b border-border/40 py-1">
      <strong className="text-foreground">{actor?.displayName ?? "…"}</strong>{" "}
      {text} · {ago(activity.createdAt)}
    </div>
  );
}

// ---------------------------------------------------------------------------
// New issue modal
// ---------------------------------------------------------------------------

function NewIssueModal({
  open,
  teams,
  currentView,
  onClose,
  onCreated,
}: {
  open: boolean;
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
    state: "todo" as StateId,
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setForm((f) => ({ ...f, teamId: defaultTeamId, title: "", description: "" }));
      setErr(null);
    }
  }, [open, defaultTeamId]);

  async function save() {
    if (!form.title.trim() || !form.teamId) return;
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>New issue</DialogTitle>
          <DialogDescription>
            <Kbd>⌘</Kbd>
            <Kbd>Enter</Kbd> to submit
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Team">
            <Select
              value={form.teamId}
              onValueChange={(v) => setForm({ ...form, teamId: v })}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {teams.map((t) => (
                  <SelectItem key={t.id} value={t.id}>
                    {t.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
          <FormField label="Priority">
            <Select
              value={String(form.priority)}
              onValueChange={(v) =>
                setForm({ ...form, priority: parseInt(v, 10) })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PRIORITIES.map((p) => (
                  <SelectItem key={p.id} value={String(p.id)}>
                    {p.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
        </div>
        <FormField label="Status">
          <Select
            value={form.state}
            onValueChange={(v) => setForm({ ...form, state: v as StateId })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {STATES.map((s) => (
                <SelectItem key={s.id} value={s.id}>
                  <div className="flex items-center gap-2">
                    <StateIcon state={s.id} />
                    {s.label}
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <FormField label="Title">
          <Input
            autoFocus
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") void save();
            }}
            placeholder="What needs to be done?"
          />
        </FormField>
        <FormField label="Description">
          <Textarea
            value={form.description}
            onChange={(e) => setForm({ ...form, description: e.target.value })}
            placeholder="Add detail, context, acceptance criteria…"
            rows={4}
          />
        </FormField>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => void save()}
            disabled={busy || !form.title.trim() || !form.teamId}
          >
            {busy && <Loader2 className="size-4 animate-spin" />}
            Create issue
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// New team modal
// ---------------------------------------------------------------------------

function NewTeamModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (teamId: string) => void;
}) {
  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setName("");
      setKey("");
      setErr(null);
    }
  }, [open]);

  useEffect(() => {
    setKey(name.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 5));
  }, [name]);

  async function save() {
    if (!name.trim() || !key.trim()) return;
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New team</DialogTitle>
          <DialogDescription>
            Issues get a per-team number prefix like ENG-42.
          </DialogDescription>
        </DialogHeader>
        <FormField label="Name">
          <Input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void save()}
            placeholder="Engineering"
          />
        </FormField>
        <FormField label="Key (1–10 uppercase)">
          <Input
            value={key}
            onChange={(e) =>
              setKey(
                e.target.value
                  .toUpperCase()
                  .replace(/[^A-Z0-9]/g, "")
                  .slice(0, 10),
              )
            }
            className="font-mono"
            placeholder="ENG"
          />
        </FormField>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => void save()}
            disabled={busy || !name.trim() || !key.trim()}
          >
            {busy && <Loader2 className="size-4 animate-spin" />}
            Create team
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

function CommandPalette({
  open,
  teams,
  onClose,
  onOpenIssue,
  onGoToTeam,
}: {
  open: boolean;
  teams: Team[];
  onClose: () => void;
  onOpenIssue: (id: string) => void;
  onGoToTeam: (id: string) => void;
}) {
  const { data: issues } = db.useQuery<Issue>("Issue");
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSel(0);
    }
  }, [open]);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const teamById = new Map(teams.map((t) => [t.id, t]));
    const out: {
      kind: "issue" | "team";
      id: string;
      label: string;
      meta: string;
    }[] = [];
    for (const t of teams) {
      if (
        !q ||
        t.name.toLowerCase().includes(q) ||
        t.key.toLowerCase().includes(q)
      ) {
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
      if (out.length >= 50) break;
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl gap-0 p-0">
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <Search className="size-4 text-muted-foreground" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKey}
            placeholder="Jump to issue, team…"
            className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          />
          <Kbd>Esc</Kbd>
        </div>
        <div className="max-h-80 overflow-y-auto p-1">
          {items.length === 0 ? (
            <div className="p-8 text-center text-sm text-muted-foreground">
              No matches.
            </div>
          ) : (
            items.map((it, i) => (
              <div
                key={`${it.kind}:${it.id}`}
                onClick={() => {
                  if (it.kind === "issue") onOpenIssue(it.id);
                  else onGoToTeam(it.id);
                }}
                onMouseEnter={() => setSel(i)}
                className={cn(
                  "flex cursor-pointer items-center gap-3 rounded-md px-3 py-2 text-sm",
                  i === sel && "bg-accent text-accent-foreground",
                )}
              >
                <span className="w-16 shrink-0 font-mono text-[11px] text-muted-foreground">
                  {it.meta}
                </span>
                <span className="flex-1 truncate">{it.label}</span>
                <Badge variant="outline" className="text-[10px]">
                  {it.kind === "issue" ? "Issue" : "Team"}
                </Badge>
              </div>
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// OrgGate + onboarding
// ---------------------------------------------------------------------------

function OrgGate({
  currentUser,
  activeOrgId,
  onSelectOrg,
  onSignOut,
  children,
}: {
  currentUser: UserRow;
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
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeOrgId, myOrgs.length]);

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
  currentUser: UserRow;
  myOrgs: Organization[];
  onSelectOrg: (orgId: string) => Promise<void>;
  onSignOut: () => void;
}) {
  const [open, setOpen] = useState(myOrgs.length === 0);
  return (
    <div className="grid min-h-screen place-items-center p-6">
      <Card className="w-[min(480px,92vw)] p-7">
        <BrandRow />
        <h2 className="mt-5 text-xl font-semibold">
          Hi, {currentUser.displayName}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Pick a workspace or create one.
        </p>
        <div className="mt-4 flex flex-col gap-1.5">
          {myOrgs.map((o) => (
            <button
              key={o.id}
              onClick={() => void onSelectOrg(o.id)}
              className="flex items-center gap-3 rounded-md border bg-secondary/40 px-3 py-2.5 text-left text-sm transition-colors hover:bg-accent"
            >
              <Avatar className="size-7 bg-primary/20">
                <AvatarFallback className="bg-transparent text-xs">
                  {initials(o.name)}
                </AvatarFallback>
              </Avatar>
              <span className="font-medium">{o.name}</span>
            </button>
          ))}
        </div>
        <div className="mt-4 flex gap-2">
          <Button onClick={() => setOpen(true)}>
            <Plus className="size-4" />
            Create workspace
          </Button>
          <Button variant="outline" onClick={onSignOut}>
            Sign out
          </Button>
        </div>
      </Card>
      <CreateOrgModal
        open={open}
        onClose={() => setOpen(false)}
        onCreated={(orgId) => {
          setOpen(false);
          void onSelectOrg(orgId);
        }}
      />
    </div>
  );
}

function CreateOrgModal({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
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
    if (!name.trim() || !slug.trim()) return;
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create workspace</DialogTitle>
        </DialogHeader>
        <FormField label="Name">
          <Input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void save()}
            placeholder="Acme Eng"
          />
        </FormField>
        <FormField label="URL slug">
          <Input
            value={slug}
            onChange={(e) => setSlug(e.target.value.toLowerCase())}
            placeholder="acme-eng"
          />
        </FormField>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() => void save()}
            disabled={busy || !name.trim() || !slug.trim()}
          >
            {busy && <Loader2 className="size-4 animate-spin" />}
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Login
// ---------------------------------------------------------------------------

function Login({ onReady }: { onReady: (u: UserRow) => void }) {
  const [email, setEmail] = useState("eng@acme.example");
  const [name, setName] = useState("Engineer");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function go() {
    if (!email.trim() || !name.trim()) return;
    setLoading(true);
    setErr(null);
    try {
      const session = (await fetch(`${BASE_URL}/api/auth/guest`, {
        method: "POST",
      }).then((r) => r.json())) as { token: string };
      const token = session.token;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL, appName: "linear" });
      const user = await callFn<UserRow>("upsertUser", {
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
    <div className="grid min-h-screen lg:grid-cols-2">
      <div className="flex items-center justify-center p-10">
        <div className="w-full max-w-sm">
          <BrandRow />
          <h1 className="mt-6 text-2xl font-bold tracking-tight">Sign in</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Track work, ship software.
          </p>
          <div className="mt-5 flex flex-col gap-3">
            <FormField label="Email">
              <Input
                autoFocus
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                type="email"
                placeholder="you@example.com"
              />
            </FormField>
            <FormField label="Display name">
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void go()}
                placeholder="Your name"
              />
            </FormField>
            {err && <ErrorBlock message={err} />}
            <Button onClick={() => void go()} disabled={loading} className="mt-2 w-full">
              {loading && <Loader2 className="size-4 animate-spin" />}
              Continue
            </Button>
          </div>
        </div>
      </div>
      <div className="hidden bg-gradient-to-br from-primary/30 via-primary/10 to-background lg:flex lg:items-center lg:p-12">
        <div className="max-w-md">
          <h2 className="text-3xl font-semibold leading-tight">
            Issues that
            <br />
            sync as you type.
          </h2>
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
            Linear-style issue tracking with team prefixes, priorities, status
            workflows, and a keyboard-driven UI. Multi-tenant under the hood.
          </p>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Shared primitives
// ---------------------------------------------------------------------------

function FormField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-1.5">
      <FormLabel>{label}</FormLabel>
      {children}
    </div>
  );
}

function ErrorBlock({ message }: { message: string }) {
  return (
    <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
      {message}
    </div>
  );
}

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded border bg-muted px-1 font-mono text-[10px] text-muted-foreground">
      {children}
    </kbd>
  );
}

function BrandMark() {
  return (
    <div className="grid size-7 place-items-center rounded-md bg-primary text-primary-foreground">
      <Box className="size-3.5" />
    </div>
  );
}

function BrandRow() {
  return (
    <div className="flex items-center gap-2">
      <BrandMark />
      <span className="text-base font-semibold">Pylon Linear</span>
    </div>
  );
}
