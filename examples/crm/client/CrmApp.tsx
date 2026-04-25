/**
 * Pylon CRM demo — companies, people, deals, notes.
 *
 * Org-scoped via tenant_id; each list is a split view (table left,
 * detail panel right). Deals get a Kanban pipeline view.
 */
import React, { useEffect, useMemo, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import {
  Building2,
  Circle,
  DollarSign,
  Loader2,
  LogOut,
  Plus,
  Users,
} from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Input } from "@pylonsync/example-ui/input";
import { Label } from "@pylonsync/example-ui/label";
import { Textarea } from "@pylonsync/example-ui/textarea";
import { Card, CardContent } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Avatar, AvatarFallback } from "@pylonsync/example-ui/avatar";
import { Separator } from "@pylonsync/example-ui/separator";
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

const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
const WS_URL =
  import.meta.env.VITE_PYLON_WS_URL ??
  (BASE_URL.startsWith("https://")
    ? `${BASE_URL.replace(/^https:/, "wss:").replace(/\/$/, "")}:4322`
    : undefined);
init({ baseUrl: BASE_URL, appName: "crm", wsUrl: WS_URL });
configureClient({ baseUrl: BASE_URL, appName: "crm" });

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type User = { id: string; email: string; displayName: string; avatarColor: string };
type Organization = {
  id: string; name: string; slug: string;
  createdBy: string; createdAt: string;
};
type OrgMember = {
  id: string; userId: string; orgId: string;
  role: string; joinedAt: string;
};
type Company = {
  id: string; orgId: string; name: string;
  domain?: string | null; industry?: string | null;
  sizeBucket?: string | null; status: string;
  description?: string | null; ownerId?: string | null;
  createdAt: string; updatedAt: string;
};
type Person = {
  id: string; orgId: string; firstName: string;
  lastName?: string | null; email?: string | null;
  phone?: string | null; title?: string | null;
  companyId?: string | null; ownerId?: string | null;
  createdAt: string; updatedAt: string;
};
type Deal = {
  id: string; orgId: string; name: string;
  companyId?: string | null; personId?: string | null;
  stage: string; amount: number; probability: number;
  closeDate?: string | null; ownerId?: string | null;
  description?: string | null;
  createdAt: string; updatedAt: string;
};
type Note = {
  id: string; orgId: string;
  targetType: string; targetId: string;
  body: string; authorId: string; createdAt: string;
};
type Activity = {
  id: string; orgId: string;
  targetType: string; targetId: string;
  kind: string; metaJson?: string | null;
  actorId: string; createdAt: string;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function initials(name: string | undefined | null): string {
  if (!name) return "·";
  const parts = name.trim().split(/\s+/);
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}
function money(n: number): string {
  return n.toLocaleString(undefined, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0,
  });
}
function ago(iso: string): string {
  const d = new Date(iso);
  const diff = Date.now() - d.getTime();
  const m = Math.floor(diff / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  const days = Math.floor(h / 24);
  if (days < 7) return `${days}d ago`;
  return d.toLocaleDateString();
}
function fullName(p: Person): string {
  return `${p.firstName}${p.lastName ? ` ${p.lastName}` : ""}`;
}

const STAGES = ["lead", "qualified", "proposal", "negotiation", "won", "lost"] as const;
const STAGE_LABELS: Record<string, string> = {
  lead: "Lead",
  qualified: "Qualified",
  proposal: "Proposal",
  negotiation: "Negotiation",
  won: "Won",
  lost: "Lost",
};
const STAGE_VARIANT: Record<string, "default" | "secondary" | "warning" | "success" | "destructive"> = {
  lead: "secondary",
  qualified: "default",
  proposal: "default",
  negotiation: "warning",
  won: "success",
  lost: "destructive",
};

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

type Page = "companies" | "people" | "deals";

export function CrmApp() {
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
    localStorage.getItem(storageKey("active_org")) || null,
  );
  const [page, setPage] = useState<Page>("companies");

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
      indexedDB.deleteDatabase("pylon_sync_crm");
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
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error?.message || `switch failed (${res.status})`);
    }
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
        <div className="grid h-screen grid-rows-[52px_1fr]">
          <Topbar currentUser={currentUser} activeOrg={org} onSignOut={signOut} />
          <div className="grid grid-cols-[200px_1fr] overflow-hidden">
            <Sidebar page={page} onNavigate={setPage} />
            <main className="flex overflow-hidden">
              {page === "companies" && <CompaniesPage org={org} />}
              {page === "people" && <PeoplePage org={org} />}
              {page === "deals" && <DealsPage org={org} />}
            </main>
          </div>
        </div>
      )}
    </OrgGate>
  );
}

// ---------------------------------------------------------------------------
// Auth + Onboarding
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
  const [createOpen, setCreateOpen] = useState(myOrgs.length === 0);
  return (
    <div className="grid min-h-screen place-items-center p-6">
      <Card className="w-[min(480px,92vw)] p-7">
        <BrandRow />
        <h2 className="mt-5 text-xl font-semibold">
          Hi, {currentUser.displayName}
        </h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Pick a workspace or create a new one.
        </p>
        <div className="mt-4 flex flex-col gap-1.5">
          {myOrgs.map((org) => (
            <button
              key={org.id}
              onClick={() => void onSelectOrg(org.id)}
              className="flex items-center gap-3 rounded-md border bg-secondary/40 px-3 py-2.5 text-left text-sm transition-colors hover:bg-accent"
            >
              <Avatar className="size-7 bg-primary/20">
                <AvatarFallback className="bg-transparent text-xs">
                  {initials(org.name)}
                </AvatarFallback>
              </Avatar>
              <div className="flex-1">
                <div className="font-medium">{org.name}</div>
                <div className="text-xs text-muted-foreground">{org.slug}</div>
              </div>
            </button>
          ))}
        </div>
        <div className="mt-4 flex gap-2">
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" />
            Create workspace
          </Button>
          <Button variant="outline" onClick={onSignOut}>
            Sign out
          </Button>
        </div>
      </Card>
      <CreateOrgModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={async (id) => {
          setCreateOpen(false);
          await db.sync.pull();
          await onSelectOrg(id);
        }}
      />
    </div>
  );
}

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit() {
    const trimmedEmail = email.trim().toLowerCase();
    if (!trimmedEmail.includes("@")) {
      setErr("Enter a valid email address.");
      return;
    }
    if (password.length < 8) {
      setErr("Password must be at least 8 characters.");
      return;
    }
    if (mode === "signup" && !displayName.trim()) {
      setErr("Enter a display name for your teammates to see.");
      return;
    }

    setLoading(true);
    setErr(null);
    try {
      const endpoint =
        mode === "signin" ? "/api/auth/password/login" : "/api/auth/password/register";
      const payload: Record<string, string> = { email: trimmedEmail, password };
      if (mode === "signup") payload.displayName = displayName.trim();

      const res = await fetch(`${BASE_URL}${endpoint}`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const body = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw new Error(
          body.error?.message ??
            (mode === "signin" ? "Sign-in failed" : "Sign-up failed"),
        );
      }
      const token: string = body.token;
      const userId: string = body.user_id;
      localStorage.setItem(storageKey("token"), token);
      configureClient({ baseUrl: BASE_URL, appName: "crm" });
      const userRes = await fetch(`${BASE_URL}/api/entities/User/${userId}`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      const user = (await userRes.json()) as User;
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
          <h1 className="mt-6 text-2xl font-bold tracking-tight">
            {mode === "signin" ? "Sign in" : "Create your account"}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {mode === "signin"
              ? "Email and password for your workspace."
              : "Set up a CRM workspace in under a minute."}
          </p>
          <div className="mt-5 flex flex-col gap-3">
            <FormField label="Email">
              <Input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="you@company.com"
                autoFocus
                onKeyDown={(e) => e.key === "Enter" && submit()}
              />
            </FormField>
            {mode === "signup" && (
              <FormField label="Display name">
                <Input
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="How your teammates will see you"
                  onKeyDown={(e) => e.key === "Enter" && submit()}
                />
              </FormField>
            )}
            <FormField label="Password">
              <Input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={mode === "signup" ? "At least 8 characters" : "••••••••"}
                onKeyDown={(e) => e.key === "Enter" && submit()}
              />
            </FormField>
            {err && <ErrorBlock message={err} />}
            <Button onClick={submit} disabled={loading} className="mt-2 w-full">
              {loading && <Loader2 className="size-4 animate-spin" />}
              {mode === "signin"
                ? loading
                  ? "Signing in…"
                  : "Sign in"
                : loading
                ? "Creating account…"
                : "Create account"}
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                setMode(mode === "signin" ? "signup" : "signin");
                setErr(null);
              }}
              disabled={loading}
              className="w-full"
            >
              {mode === "signin"
                ? "Need an account? Sign up"
                : "Already have an account? Sign in"}
            </Button>
          </div>
        </div>
      </div>
      <div className="hidden bg-gradient-to-br from-primary/30 via-primary/10 to-background lg:flex lg:items-center lg:p-12">
        <div className="max-w-md">
          <h2 className="text-3xl font-semibold leading-tight">
            Customers, deals,
            <br />
            and the relationships between them.
          </h2>
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
            Companies, people, and deals — every change syncs across teammates
            in real time. Built on Pylon.
          </p>
        </div>
      </div>
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
  onCreated: (id: string) => void;
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
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create workspace</DialogTitle>
          <DialogDescription>Container for your CRM data.</DialogDescription>
        </DialogHeader>
        <FormField label="Name">
          <Input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Acme Sales"
          />
        </FormField>
        <FormField label="URL slug">
          <Input
            value={slug}
            onChange={(e) => setSlug(e.target.value.toLowerCase())}
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
// Topbar + Sidebar
// ---------------------------------------------------------------------------

function Topbar({
  currentUser,
  activeOrg,
  onSignOut,
}: {
  currentUser: User;
  activeOrg: Organization;
  onSignOut: () => void;
}) {
  return (
    <header className="flex items-center gap-4 border-b bg-card/60 px-5">
      <BrandRow />
      <Badge variant="secondary">{activeOrg.name}</Badge>
      <div className="flex-1" />
      <div className="flex items-center gap-2 rounded-full border bg-card px-2 py-0.5">
        <Avatar
          className="size-6"
          style={{ backgroundColor: currentUser.avatarColor }}
        >
          <AvatarFallback className="bg-transparent text-[10px] text-white">
            {initials(currentUser.displayName)}
          </AvatarFallback>
        </Avatar>
        <span className="text-xs">{currentUser.displayName}</span>
        <Button
          variant="ghost"
          size="xs"
          onClick={onSignOut}
          className="text-muted-foreground"
        >
          <LogOut className="size-3" />
          Sign out
        </Button>
      </div>
    </header>
  );
}

function Sidebar({
  page,
  onNavigate,
}: {
  page: Page;
  onNavigate: (p: Page) => void;
}) {
  const items: { id: Page; label: string; icon: React.ReactNode }[] = [
    { id: "companies", label: "Companies", icon: <Building2 className="size-4" /> },
    { id: "people", label: "People", icon: <Users className="size-4" /> },
    { id: "deals", label: "Deals", icon: <DollarSign className="size-4" /> },
  ];
  return (
    <nav className="flex flex-col gap-1 border-r bg-card/40 p-2">
      {items.map((it) => (
        <button
          key={it.id}
          onClick={() => onNavigate(it.id)}
          className={cn(
            "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors",
            page === it.id
              ? "bg-accent text-accent-foreground"
              : "text-foreground/80 hover:bg-accent/50",
          )}
        >
          {it.icon}
          {it.label}
        </button>
      ))}
    </nav>
  );
}

// ---------------------------------------------------------------------------
// Companies page
// ---------------------------------------------------------------------------

function CompaniesPage({ org }: { org: Organization }) {
  const { data: companies } = db.useQuery<Company>("Company", {
    where: { orgId: org.id },
    orderBy: { name: "asc" },
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const list = companies ?? [];
  const selected = list.find((c) => c.id === selectedId) ?? null;

  useEffect(() => {
    if (!selectedId && list.length > 0) setSelectedId(list[0].id);
  }, [list, selectedId]);

  return (
    <div className="grid w-full grid-cols-[1fr_360px] overflow-hidden">
      <div className="flex flex-col overflow-hidden">
        <ListHeader
          title="Companies"
          count={list.length}
          actionLabel="Add company"
          onAction={() => setAddOpen(true)}
        />
        {list.length === 0 ? (
          <EmptyState
            title="No companies yet"
            sub="Add your first account to get started."
            action={
              <Button onClick={() => setAddOpen(true)}>
                <Plus className="size-4" />
                Add company
              </Button>
            }
          />
        ) : (
          <div className="overflow-y-auto">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-card/95 backdrop-blur">
                <tr className="border-b text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  <th className="px-4 py-2 text-left">Name</th>
                  <th className="px-4 py-2 text-left">Industry</th>
                  <th className="px-4 py-2 text-left">Size</th>
                  <th className="px-4 py-2 text-left">Status</th>
                  <th className="px-4 py-2 text-left">Added</th>
                </tr>
              </thead>
              <tbody>
                {list.map((c) => (
                  <tr
                    key={c.id}
                    onClick={() => setSelectedId(c.id)}
                    className={cn(
                      "cursor-pointer border-b border-border/40 transition-colors hover:bg-muted/30",
                      c.id === selectedId && "bg-accent",
                    )}
                  >
                    <td className="px-4 py-2.5">
                      <div className="flex items-center gap-2">
                        <Avatar className="size-7 bg-amber-200">
                          <AvatarFallback className="bg-transparent text-[10px]">
                            {initials(c.name)}
                          </AvatarFallback>
                        </Avatar>
                        <span className="font-medium">{c.name}</span>
                      </div>
                    </td>
                    <td className="px-4 py-2.5 text-muted-foreground">
                      {c.industry || "—"}
                    </td>
                    <td className="px-4 py-2.5 text-muted-foreground">
                      {c.sizeBucket || "—"}
                    </td>
                    <td className="px-4 py-2.5">
                      <Badge variant="secondary" className="capitalize">
                        {c.status}
                      </Badge>
                    </td>
                    <td className="px-4 py-2.5 text-muted-foreground">
                      {ago(c.createdAt)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <DetailPane>
        {selected ? (
          <CompanyDetail company={selected} />
        ) : (
          <EmptyDetail>Pick a company to see details.</EmptyDetail>
        )}
      </DetailPane>
      <AddCompanyModal open={addOpen} onClose={() => setAddOpen(false)} />
    </div>
  );
}

function CompanyDetail({ company }: { company: Company }) {
  const { data: people } = db.useQuery<Person>("Person", {
    where: { companyId: company.id },
  });
  return (
    <>
      <DetailHeader
        avatar={
          <Avatar className="size-12 bg-amber-200">
            <AvatarFallback className="bg-transparent">
              {initials(company.name)}
            </AvatarFallback>
          </Avatar>
        }
        title={company.name}
        sub={`${company.domain || "—"}${company.industry ? ` · ${company.industry}` : ""}`}
      />
      <DetailSection title="Details">
        <KvList
          items={[
            ["Status", <Badge variant="secondary" className="capitalize">{company.status}</Badge>],
            ["Size", company.sizeBucket || "—"],
            ["Domain", company.domain || "—"],
            ["Created", ago(company.createdAt)],
          ]}
        />
      </DetailSection>
      <DetailSection title={`People · ${(people ?? []).length}`}>
        {(people ?? []).length === 0 ? (
          <p className="text-xs text-muted-foreground">No people linked yet.</p>
        ) : (
          <div className="flex flex-col gap-1.5">
            {(people ?? []).map((p) => (
              <div key={p.id} className="flex items-center gap-2 text-sm">
                <Avatar className="size-5 bg-primary/20">
                  <AvatarFallback className="bg-transparent text-[9px]">
                    {initials(fullName(p))}
                  </AvatarFallback>
                </Avatar>
                <span className="font-medium">{fullName(p)}</span>
                {p.title && (
                  <span className="text-xs text-muted-foreground">
                    {p.title}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </DetailSection>
      <NotesSection targetType="Company" targetId={company.id} />
      <TimelineSection targetType="Company" targetId={company.id} />
    </>
  );
}

function AddCompanyModal({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [form, setForm] = useState({
    name: "",
    domain: "",
    industry: "",
    sizeBucket: "",
    status: "lead",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setForm({ name: "", domain: "", industry: "", sizeBucket: "", status: "lead" });
      setErr(null);
    }
  }, [open]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createCompany", {
        name: form.name,
        domain: form.domain || undefined,
        industry: form.industry || undefined,
        sizeBucket: form.sizeBucket || undefined,
        status: form.status,
      });
      onClose();
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
          <DialogTitle>Add company</DialogTitle>
          <DialogDescription>Track a new account.</DialogDescription>
        </DialogHeader>
        <FormField label="Name">
          <Input
            autoFocus
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder="Acme Corp"
          />
        </FormField>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Domain">
            <Input
              value={form.domain}
              onChange={(e) => setForm({ ...form, domain: e.target.value })}
              placeholder="acme.com"
            />
          </FormField>
          <FormField label="Industry">
            <Input
              value={form.industry}
              onChange={(e) => setForm({ ...form, industry: e.target.value })}
            />
          </FormField>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Size">
            <Select
              value={form.sizeBucket || "__none__"}
              onValueChange={(v) =>
                setForm({ ...form, sizeBucket: v === "__none__" ? "" : v })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">—</SelectItem>
                <SelectItem value="1-10">1–10</SelectItem>
                <SelectItem value="11-50">11–50</SelectItem>
                <SelectItem value="51-200">51–200</SelectItem>
                <SelectItem value="201-500">201–500</SelectItem>
                <SelectItem value="500+">500+</SelectItem>
              </SelectContent>
            </Select>
          </FormField>
          <FormField label="Status">
            <Select
              value={form.status}
              onValueChange={(v) => setForm({ ...form, status: v })}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="lead">Lead</SelectItem>
                <SelectItem value="active">Active</SelectItem>
                <SelectItem value="churned">Churned</SelectItem>
              </SelectContent>
            </Select>
          </FormField>
        </div>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={busy || !form.name.trim()}>
            {busy && <Loader2 className="size-4 animate-spin" />}
            Add company
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// People page
// ---------------------------------------------------------------------------

function PeoplePage({ org }: { org: Organization }) {
  const { data: people } = db.useQuery<Person>("Person", {
    where: { orgId: org.id },
    orderBy: { firstName: "asc" },
  });
  const { data: companies } = db.useQuery<Company>("Company", {
    where: { orgId: org.id },
  });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const list = people ?? [];
  const companyById = new Map((companies ?? []).map((c) => [c.id, c]));
  const selected = list.find((p) => p.id === selectedId) ?? null;

  useEffect(() => {
    if (!selectedId && list.length > 0) setSelectedId(list[0].id);
  }, [list, selectedId]);

  return (
    <div className="grid w-full grid-cols-[1fr_360px] overflow-hidden">
      <div className="flex flex-col overflow-hidden">
        <ListHeader
          title="People"
          count={list.length}
          actionLabel="Add person"
          onAction={() => setAddOpen(true)}
        />
        {list.length === 0 ? (
          <EmptyState
            title="No people yet"
            sub="Add a contact to link to a company."
            action={
              <Button onClick={() => setAddOpen(true)}>
                <Plus className="size-4" />
                Add person
              </Button>
            }
          />
        ) : (
          <div className="overflow-y-auto">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-card/95 backdrop-blur">
                <tr className="border-b text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  <th className="px-4 py-2 text-left">Name</th>
                  <th className="px-4 py-2 text-left">Title</th>
                  <th className="px-4 py-2 text-left">Company</th>
                  <th className="px-4 py-2 text-left">Email</th>
                </tr>
              </thead>
              <tbody>
                {list.map((p) => {
                  const company = p.companyId
                    ? companyById.get(p.companyId)
                    : undefined;
                  return (
                    <tr
                      key={p.id}
                      onClick={() => setSelectedId(p.id)}
                      className={cn(
                        "cursor-pointer border-b border-border/40 transition-colors hover:bg-muted/30",
                        p.id === selectedId && "bg-accent",
                      )}
                    >
                      <td className="px-4 py-2.5">
                        <div className="flex items-center gap-2">
                          <Avatar className="size-7 bg-primary/20">
                            <AvatarFallback className="bg-transparent text-[10px]">
                              {initials(fullName(p))}
                            </AvatarFallback>
                          </Avatar>
                          <span className="font-medium">{fullName(p)}</span>
                        </div>
                      </td>
                      <td className="px-4 py-2.5 text-muted-foreground">
                        {p.title || "—"}
                      </td>
                      <td className="px-4 py-2.5">
                        {company ? (
                          <span className="inline-flex items-center gap-1.5 rounded-md border bg-secondary/50 px-2 py-0.5 text-xs">
                            <Avatar className="size-4 bg-amber-200">
                              <AvatarFallback className="bg-transparent text-[8px]">
                                {initials(company.name)}
                              </AvatarFallback>
                            </Avatar>
                            {company.name}
                          </span>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </td>
                      <td className="px-4 py-2.5 text-muted-foreground">
                        {p.email || "—"}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <DetailPane>
        {selected ? (
          <PersonDetail person={selected} companyById={companyById} />
        ) : (
          <EmptyDetail>Pick a person to see details.</EmptyDetail>
        )}
      </DetailPane>
      <AddPersonModal
        open={addOpen}
        companies={companies ?? []}
        onClose={() => setAddOpen(false)}
      />
    </div>
  );
}

function PersonDetail({
  person,
  companyById,
}: {
  person: Person;
  companyById: Map<string, Company>;
}) {
  const company = person.companyId
    ? companyById.get(person.companyId)
    : undefined;
  return (
    <>
      <DetailHeader
        avatar={
          <Avatar className="size-12 bg-primary/20">
            <AvatarFallback className="bg-transparent">
              {initials(fullName(person))}
            </AvatarFallback>
          </Avatar>
        }
        title={fullName(person)}
        sub={`${person.title || "—"}${company ? ` · ${company.name}` : ""}`}
      />
      <DetailSection title="Contact">
        <KvList
          items={[
            ["Email", person.email || "—"],
            ["Phone", person.phone || "—"],
            ["Company", company?.name || "—"],
            ["Created", ago(person.createdAt)],
          ]}
        />
      </DetailSection>
      <NotesSection targetType="Person" targetId={person.id} />
      <TimelineSection targetType="Person" targetId={person.id} />
    </>
  );
}

function AddPersonModal({
  open,
  companies,
  onClose,
}: {
  open: boolean;
  companies: Company[];
  onClose: () => void;
}) {
  const [form, setForm] = useState({
    firstName: "", lastName: "", email: "", phone: "", title: "", companyId: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setForm({ firstName: "", lastName: "", email: "", phone: "", title: "", companyId: "" });
      setErr(null);
    }
  }, [open]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createPerson", {
        firstName: form.firstName,
        lastName: form.lastName || undefined,
        email: form.email || undefined,
        phone: form.phone || undefined,
        title: form.title || undefined,
        companyId: form.companyId || undefined,
      });
      onClose();
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
          <DialogTitle>Add person</DialogTitle>
          <DialogDescription>Contact in your network.</DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="First name">
            <Input
              autoFocus
              value={form.firstName}
              onChange={(e) => setForm({ ...form, firstName: e.target.value })}
            />
          </FormField>
          <FormField label="Last name">
            <Input
              value={form.lastName}
              onChange={(e) => setForm({ ...form, lastName: e.target.value })}
            />
          </FormField>
        </div>
        <FormField label="Title">
          <Input
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
          />
        </FormField>
        <FormField label="Company">
          <Select
            value={form.companyId || "__none__"}
            onValueChange={(v) =>
              setForm({ ...form, companyId: v === "__none__" ? "" : v })
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none__">—</SelectItem>
              {companies.map((c) => (
                <SelectItem key={c.id} value={c.id}>
                  {c.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </FormField>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Email">
            <Input
              value={form.email}
              onChange={(e) => setForm({ ...form, email: e.target.value })}
            />
          </FormField>
          <FormField label="Phone">
            <Input
              value={form.phone}
              onChange={(e) => setForm({ ...form, phone: e.target.value })}
            />
          </FormField>
        </div>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={busy || !form.firstName.trim()}>
            {busy && <Loader2 className="size-4 animate-spin" />}
            Add person
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Deals page (Kanban)
// ---------------------------------------------------------------------------

function DealsPage({ org }: { org: Organization }) {
  const { data: deals } = db.useQuery<Deal>("Deal", { where: { orgId: org.id } });
  const { data: companies } = db.useQuery<Company>("Company", {
    where: { orgId: org.id },
  });
  const { data: people } = db.useQuery<Person>("Person", {
    where: { orgId: org.id },
  });
  const [addOpen, setAddOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const byStage = useMemo(() => {
    const map: Record<string, Deal[]> = Object.fromEntries(
      STAGES.map((s) => [s, []]),
    );
    for (const d of deals ?? []) {
      if (map[d.stage]) map[d.stage].push(d);
    }
    for (const s of STAGES) {
      map[s].sort((a, b) => b.amount - a.amount);
    }
    return map;
  }, [deals]);

  const companyById = new Map((companies ?? []).map((c) => [c.id, c]));
  const personById = new Map((people ?? []).map((p) => [p.id, p]));
  const selected = (deals ?? []).find((d) => d.id === selectedId) ?? null;

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <ListHeader
        title="Pipeline"
        count={(deals ?? []).length}
        actionLabel="New deal"
        onAction={() => setAddOpen(true)}
      />
      <div className="grid flex-1 grid-cols-6 gap-3 overflow-x-auto p-4">
        {STAGES.map((stage) => {
          const items = byStage[stage] ?? [];
          const total = items.reduce((s, d) => s + d.amount, 0);
          return (
            <div key={stage} className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <Badge
                  variant={STAGE_VARIANT[stage]}
                  className="capitalize"
                >
                  {STAGE_LABELS[stage]}
                </Badge>
                <span className="font-mono text-xs text-muted-foreground">
                  {items.length}
                </span>
                <span className="ml-auto font-mono text-xs text-muted-foreground">
                  {money(total)}
                </span>
              </div>
              <div className="flex flex-col gap-2 overflow-y-auto">
                {items.map((d) => (
                  <DealCard
                    key={d.id}
                    deal={d}
                    company={d.companyId ? companyById.get(d.companyId) : undefined}
                    person={d.personId ? personById.get(d.personId) : undefined}
                    onClick={() => setSelectedId(d.id)}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </div>
      <AddDealModal
        open={addOpen}
        companies={companies ?? []}
        people={people ?? []}
        onClose={() => setAddOpen(false)}
      />
      <DealModal
        deal={selected}
        company={selected?.companyId ? companyById.get(selected.companyId) : undefined}
        onClose={() => setSelectedId(null)}
      />
    </div>
  );
}

function DealCard({
  deal,
  company,
  person,
  onClick,
}: {
  deal: Deal;
  company?: Company;
  person?: Person;
  onClick: () => void;
}) {
  return (
    <Card
      onClick={onClick}
      className="cursor-pointer p-3 transition-colors hover:border-primary/40"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="text-sm font-medium leading-tight">{deal.name}</div>
        <div className="font-mono text-sm font-semibold tabular-nums">
          {money(deal.amount)}
        </div>
      </div>
      {(company || person) && (
        <div className="mt-2 flex flex-wrap items-center gap-1 text-xs">
          {company && (
            <span className="inline-flex items-center gap-1 rounded-md border bg-secondary/50 px-1.5 py-0.5">
              <Avatar className="size-3.5 bg-amber-200">
                <AvatarFallback className="bg-transparent text-[7px]">
                  {initials(company.name)}
                </AvatarFallback>
              </Avatar>
              {company.name}
            </span>
          )}
          {person && (
            <span className="text-muted-foreground">· {fullName(person)}</span>
          )}
        </div>
      )}
      <div className="mt-2 text-[11px] text-muted-foreground">
        {deal.probability}% · {ago(deal.createdAt)}
      </div>
    </Card>
  );
}

function DealModal({
  deal,
  company,
  onClose,
}: {
  deal: Deal | null;
  company?: Company;
  onClose: () => void;
}) {
  if (!deal) return null;
  async function moveTo(stage: string) {
    if (!deal) return;
    try {
      await callFn("updateDealStage", { dealId: deal.id, stage });
    } catch (e) {
      alert((e as Error).message);
    }
  }
  return (
    <Dialog open={!!deal} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>{deal.name}</DialogTitle>
          <DialogDescription>
            {money(deal.amount)} · {company?.name || "No company"} ·{" "}
            {deal.probability}%
          </DialogDescription>
        </DialogHeader>
        <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          Move to stage
        </div>
        <div className="flex flex-wrap gap-2">
          {STAGES.map((s) => (
            <button
              key={s}
              onClick={() => void moveTo(s)}
              className={cn(
                "rounded-md border px-3 py-1 text-xs font-medium transition-colors",
                s === deal.stage
                  ? "border-primary bg-primary/10 text-primary"
                  : "border-border hover:bg-accent",
              )}
            >
              {STAGE_LABELS[s]}
            </button>
          ))}
        </div>
        <NotesSection targetType="Deal" targetId={deal.id} />
        <TimelineSection targetType="Deal" targetId={deal.id} />
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AddDealModal({
  open,
  companies,
  people,
  onClose,
}: {
  open: boolean;
  companies: Company[];
  people: Person[];
  onClose: () => void;
}) {
  const [form, setForm] = useState({
    name: "", companyId: "", personId: "", stage: "lead", amount: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setForm({ name: "", companyId: "", personId: "", stage: "lead", amount: "" });
      setErr(null);
    }
  }, [open]);

  async function save() {
    setBusy(true);
    setErr(null);
    try {
      await callFn("createDeal", {
        name: form.name,
        companyId: form.companyId || undefined,
        personId: form.personId || undefined,
        stage: form.stage,
        amount: form.amount ? parseFloat(form.amount) : 0,
      });
      onClose();
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
          <DialogTitle>New deal</DialogTitle>
          <DialogDescription>Track an opportunity.</DialogDescription>
        </DialogHeader>
        <FormField label="Name">
          <Input
            autoFocus
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder="Acme — Q2 expansion"
          />
        </FormField>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Company">
            <Select
              value={form.companyId || "__none__"}
              onValueChange={(v) =>
                setForm({ ...form, companyId: v === "__none__" ? "" : v })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">—</SelectItem>
                {companies.map((c) => (
                  <SelectItem key={c.id} value={c.id}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
          <FormField label="Person">
            <Select
              value={form.personId || "__none__"}
              onValueChange={(v) =>
                setForm({ ...form, personId: v === "__none__" ? "" : v })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">—</SelectItem>
                {people.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {fullName(p)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
        </div>
        <div className="grid grid-cols-2 gap-3">
          <FormField label="Stage">
            <Select
              value={form.stage}
              onValueChange={(v) => setForm({ ...form, stage: v })}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STAGES.map((s) => (
                  <SelectItem key={s} value={s}>
                    {STAGE_LABELS[s]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </FormField>
          <FormField label="Amount (USD)">
            <Input
              type="number"
              min="0"
              step="100"
              value={form.amount}
              onChange={(e) => setForm({ ...form, amount: e.target.value })}
            />
          </FormField>
        </div>
        {err && <ErrorBlock message={err} />}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={busy || !form.name.trim()}>
            {busy && <Loader2 className="size-4 animate-spin" />}
            Create deal
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// Notes + Timeline (reusable)
// ---------------------------------------------------------------------------

function NotesSection({
  targetType,
  targetId,
}: {
  targetType: string;
  targetId: string;
}) {
  const { data: notes } = db.useQuery<Note>("Note", {
    where: { targetType, targetId },
    orderBy: { createdAt: "desc" },
  });
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);

  async function add() {
    if (!body.trim()) return;
    setBusy(true);
    try {
      await callFn("addNote", { targetType, targetId, body });
      setBody("");
    } catch (e) {
      alert((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <DetailSection title="Notes">
      <div className="mb-3 flex gap-2">
        <Input
          value={body}
          onChange={(e) => setBody(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
          placeholder="Add a note…"
        />
        <Button
          size="sm"
          disabled={busy || !body.trim()}
          onClick={() => void add()}
        >
          Add
        </Button>
      </div>
      <div className="flex flex-col gap-2">
        {(notes ?? []).map((n) => (
          <NoteCard key={n.id} note={n} />
        ))}
      </div>
    </DetailSection>
  );
}

function NoteCard({ note }: { note: Note }) {
  const { data: author } = db.useQueryOne<User>("User", note.authorId);
  return (
    <div className="rounded-md border bg-secondary/40 p-3">
      <p className="whitespace-pre-wrap text-sm">{note.body}</p>
      <div className="mt-1.5 text-[11px] text-muted-foreground">
        {author?.displayName ?? "…"} · {ago(note.createdAt)}
      </div>
    </div>
  );
}

function TimelineSection({
  targetType,
  targetId,
}: {
  targetType: string;
  targetId: string;
}) {
  const { data: activities } = db.useQuery<Activity>("Activity", {
    where: { targetType, targetId },
    orderBy: { createdAt: "desc" },
  });
  return (
    <DetailSection title="Activity">
      {(activities ?? []).length === 0 ? (
        <p className="text-xs text-muted-foreground">Nothing here yet.</p>
      ) : (
        <div className="flex flex-col gap-2">
          {(activities ?? []).map((a) => (
            <ActivityRow key={a.id} activity={a} />
          ))}
        </div>
      )}
    </DetailSection>
  );
}

function ActivityRow({ activity }: { activity: Activity }) {
  const { data: actor } = db.useQueryOne<User>("User", activity.actorId);
  let text = "";
  switch (activity.kind) {
    case "created":
      text = "created this record";
      break;
    case "stage_changed":
      try {
        const meta = JSON.parse(activity.metaJson || "{}") as {
          from?: string;
          to?: string;
        };
        text = `moved stage ${
          STAGE_LABELS[meta.from ?? ""] || meta.from
        } → ${STAGE_LABELS[meta.to ?? ""] || meta.to}`;
      } catch {
        text = "changed stage";
      }
      break;
    case "note_added":
      try {
        const meta = JSON.parse(activity.metaJson || "{}") as {
          preview?: string;
        };
        text = `added a note — "${meta.preview ?? ""}"`;
      } catch {
        text = "added a note";
      }
      break;
    default:
      text = activity.kind;
  }
  return (
    <div className="flex gap-2 text-xs">
      <Circle className="mt-1 size-2 shrink-0 fill-current text-muted-foreground" />
      <div className="flex-1">
        <div>
          <strong className="text-foreground">
            {actor?.displayName ?? "Someone"}
          </strong>{" "}
          {text}
        </div>
        <div className="text-[11px] text-muted-foreground">
          {ago(activity.createdAt)}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

function ListHeader({
  title,
  count,
  actionLabel,
  onAction,
}: {
  title: string;
  count: number;
  actionLabel: string;
  onAction: () => void;
}) {
  return (
    <header className="flex h-12 items-center gap-3 border-b px-5">
      <h1 className="text-sm font-semibold">{title}</h1>
      <Badge variant="secondary" className="font-mono text-[10px]">
        {count}
      </Badge>
      <div className="flex-1" />
      <Button size="sm" onClick={onAction}>
        <Plus className="size-4" />
        {actionLabel}
      </Button>
    </header>
  );
}

function DetailPane({ children }: { children: React.ReactNode }) {
  return (
    <aside className="flex flex-col overflow-y-auto border-l bg-card/40 p-5">
      {children}
    </aside>
  );
}

function DetailHeader({
  avatar,
  title,
  sub,
}: {
  avatar: React.ReactNode;
  title: string;
  sub: string;
}) {
  return (
    <div className="mb-4 flex items-center gap-3">
      {avatar}
      <div>
        <div className="text-base font-semibold">{title}</div>
        <div className="text-xs text-muted-foreground">{sub}</div>
      </div>
    </div>
  );
}

function DetailSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-5">
      <div className="mb-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </div>
      {children}
    </section>
  );
}

function KvList({ items }: { items: Array<[string, React.ReactNode]> }) {
  return (
    <dl className="grid grid-cols-[80px_1fr] gap-y-1.5 text-sm">
      {items.map(([k, v]) => (
        <React.Fragment key={k}>
          <dt className="text-muted-foreground">{k}</dt>
          <dd>{v}</dd>
        </React.Fragment>
      ))}
    </dl>
  );
}

function EmptyState({
  title,
  sub,
  action,
}: {
  title: string;
  sub: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="grid flex-1 place-items-center">
      <div className="flex flex-col items-center gap-3 text-center">
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="text-sm text-muted-foreground">{sub}</p>
        {action}
      </div>
    </div>
  );
}

function EmptyDetail({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid h-full place-items-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}

function FormField({
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

function ErrorBlock({ message }: { message: string }) {
  return (
    <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
      {message}
    </div>
  );
}

function BrandRow() {
  return (
    <div className="flex items-center gap-2">
      <BrandMark />
      <span className="text-base font-semibold">Pylon CRM</span>
    </div>
  );
}

function BrandMark() {
  return (
    <div className="grid size-7 place-items-center rounded-md bg-primary text-primary-foreground">
      <DollarSign className="size-3.5" />
    </div>
  );
}
