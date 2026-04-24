/**
 * Pylon CRM demo — Attio-style: companies, people, deals, notes.
 * Org-scoped via tenant_id; reuses the session + filter patterns from the
 * ERP example. Every list is a split view (table left, detail panel right).
 */

import React, { useEffect, useMemo, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";

// Vite inlines VITE_* env vars at build time. Set VITE_PYLON_URL in
// Vercel → Project Settings → Environment Variables (e.g.
// https://pylon-crm.fly.dev) so the deployed frontend talks to the real
// backend instead of localhost. Local dev falls back to localhost:4321.
const BASE_URL = import.meta.env.VITE_PYLON_URL ?? "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "crm" });
configureClient({ baseUrl: BASE_URL, appName: "crm" });

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
type Company = {
  id: string;
  orgId: string;
  name: string;
  domain?: string | null;
  industry?: string | null;
  sizeBucket?: string | null;
  status: string;
  description?: string | null;
  ownerId?: string | null;
  createdAt: string;
  updatedAt: string;
};
type Person = {
  id: string;
  orgId: string;
  firstName: string;
  lastName?: string | null;
  email?: string | null;
  phone?: string | null;
  title?: string | null;
  companyId?: string | null;
  ownerId?: string | null;
  createdAt: string;
  updatedAt: string;
};
type Deal = {
  id: string;
  orgId: string;
  name: string;
  companyId?: string | null;
  personId?: string | null;
  stage: string;
  amount: number;
  probability: number;
  closeDate?: string | null;
  ownerId?: string | null;
  description?: string | null;
  createdAt: string;
  updatedAt: string;
};
type Note = {
  id: string;
  orgId: string;
  targetType: string;
  targetId: string;
  body: string;
  authorId: string;
  createdAt: string;
};
type Activity = {
  id: string;
  orgId: string;
  targetType: string;
  targetId: string;
  kind: string;
  metaJson?: string | null;
  actorId: string;
  createdAt: string;
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

const STAGES = ["lead", "qualified", "proposal", "negotiation", "won", "lost"];
const STAGE_LABELS: Record<string, string> = {
  lead: "Lead",
  qualified: "Qualified",
  proposal: "Proposal",
  negotiation: "Negotiation",
  won: "Won",
  lost: "Lost",
};
const STAGE_COLORS: Record<string, string> = {
  lead: "pill-gray",
  qualified: "pill-accent",
  proposal: "pill-accent",
  negotiation: "pill-warning",
  won: "pill-success",
  lost: "pill-danger",
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
  const [activeOrgId, setActiveOrgId] = useState<string | null>(() => {
    return localStorage.getItem(storageKey("active_org")) || null;
  });
  const [page, setPage] = useState<Page>("companies");

  useEffect(() => {
    if (currentUser) void db.sync.pull();
  }, [currentUser?.id]);

  // Reconcile server's tenant with our local activeOrgId — same pattern
  // as ERP so new sessions don't drift.
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
        if (cancelled) return;
        if (me.tenant_id === activeOrgId) return;
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
        <div className="app">
          <Topbar
            currentUser={currentUser}
            activeOrg={org}
            onSignOut={signOut}
          />
          <div className="body">
            <Sidebar page={page} onNavigate={setPage} />
            <main className="main">
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
// OrgGate + Login + Onboarding
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
    <div className="split-screen">
      <div className="auth-panel">
        <div className="brand" style={{ marginBottom: 20 }}>
          <BrandMark />
          Pylon CRM
        </div>
        <div className="auth-title">Hi, {currentUser.displayName}</div>
        <div className="auth-subtitle">
          Pick a workspace or create a new one.
        </div>
        {myOrgs.length > 0 && (
          <>
            {myOrgs.map((org) => (
              <button
                key={org.id}
                onClick={() => void onSelectOrg(org.id)}
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
                  {initials(org.name)}
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontWeight: 500 }}>{org.name}</div>
                  <div style={{ fontSize: 11, color: "var(--text-dim)" }}>
                    {org.slug}
                  </div>
                </div>
              </button>
            ))}
          </>
        )}
        <div style={{ display: "flex", gap: 8, marginTop: 16 }}>
          <button className="btn btn-primary" onClick={() => setCreateOpen(true)}>
            Create workspace
          </button>
          <button className="btn btn-ghost" onClick={onSignOut}>
            Sign out
          </button>
        </div>
      </div>
      {createOpen && (
        <CreateOrgModal
          onClose={() => setCreateOpen(false)}
          onCreated={async (id) => {
            // Pull the replica before switching — createOrganization
            // inserts Organization + OrgMember server-side, but our local
            // useQuery<OrgMember> won't include the new row until sync
            // catches up. Without this, select-org flips activeOrgId to
            // an id that's not in myOrgs, and OrgGate falls back to the
            // onboarding screen.
            setCreateOpen(false);
            await db.sync.pull();
            await onSelectOrg(id);
          }}
        />
      )}
    </div>
  );
}

function Login({ onReady }: { onReady: (u: User) => void }) {
  const [email, setEmail] = useState("sales@acme.example");
  const [name, setName] = useState("Sales Lead");
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
      configureClient({ baseUrl: BASE_URL, appName: "crm" });
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
        <div className="brand" style={{ marginBottom: 20 }}>
          <BrandMark />
          Pylon CRM
        </div>
        <div className="auth-title">Sign in</div>
        <div className="auth-subtitle">Track companies, people, and deals.</div>
        <label className="field">
          <span className="field-label">Email</span>
          <input
            className="input"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            autoFocus
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

function CreateOrgModal({
  onClose,
  onCreated,
}: {
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
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Create workspace</div>
        <div className="modal-subtitle">Container for your CRM data.</div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
            autoFocus
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Acme Sales"
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
    <header className="topbar">
      <div className="brand">
        <BrandMark />
        Pylon CRM
      </div>
      <div
        className="pill pill-gray"
        style={{ padding: "3px 10px", fontSize: 12 }}
      >
        {activeOrg.name}
      </div>
      <div className="spacer" />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "2px 10px 2px 2px",
          background: "var(--surface-hover)",
          border: "1px solid var(--border)",
          borderRadius: 999,
        }}
      >
        <div
          className="avatar avatar-sm"
          style={{ backgroundColor: currentUser.avatarColor }}
        >
          {initials(currentUser.displayName)}
        </div>
        <span style={{ fontSize: 12 }}>{currentUser.displayName}</span>
        <button
          onClick={onSignOut}
          className="btn btn-ghost"
          style={{ padding: "2px 8px", fontSize: 11 }}
        >
          Sign out
        </button>
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
    { id: "companies", label: "Companies", icon: <IconBuilding /> },
    { id: "people", label: "People", icon: <IconUser /> },
    { id: "deals", label: "Deals", icon: <IconMoney /> },
  ];
  return (
    <nav className="nav">
      {items.map((it) => (
        <div
          key={it.id}
          className={"nav-item" + (page === it.id ? " active" : "")}
          onClick={() => onNavigate(it.id)}
        >
          {it.icon}
          {it.label}
        </div>
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
    <>
      <div className="list-pane">
        <div className="list-header">
          <div className="list-title">Companies</div>
          <div className="list-count">{list.length}</div>
          <div className="spacer" />
          <button
            className="btn btn-primary"
            onClick={() => setAddOpen(true)}
          >
            <IconPlus /> Add company
          </button>
        </div>
        {list.length === 0 ? (
          <div className="empty">
            <div className="empty-title">No companies yet</div>
            <div className="empty-body">
              Add your first account to get started.
            </div>
            <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
              Add company
            </button>
          </div>
        ) : (
          <div style={{ overflowY: "auto", flex: 1 }}>
            <table className="table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Industry</th>
                  <th>Size</th>
                  <th>Status</th>
                  <th>Added</th>
                </tr>
              </thead>
              <tbody>
                {list.map((c) => (
                  <tr
                    key={c.id}
                    className={c.id === selectedId ? "selected" : ""}
                    onClick={() => setSelectedId(c.id)}
                  >
                    <td>
                      <div
                        style={{ display: "flex", alignItems: "center", gap: 8 }}
                      >
                        <div
                          className="avatar avatar-sm"
                          style={{ backgroundColor: "#fde68a" }}
                        >
                          {initials(c.name)}
                        </div>
                        <span style={{ fontWeight: 500 }}>{c.name}</span>
                      </div>
                    </td>
                    <td style={{ color: "var(--text-muted)" }}>
                      {c.industry || "—"}
                    </td>
                    <td style={{ color: "var(--text-muted)" }}>
                      {c.sizeBucket || "—"}
                    </td>
                    <td>
                      <span className="pill pill-gray">{c.status}</span>
                    </td>
                    <td style={{ color: "var(--text-muted)" }}>
                      {ago(c.createdAt)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
      <aside className="detail-pane">
        {selected ? (
          <CompanyDetail company={selected} />
        ) : (
          <div className="empty">
            <div className="empty-body">Pick a company to see details.</div>
          </div>
        )}
      </aside>
      {addOpen && <AddCompanyModal onClose={() => setAddOpen(false)} />}
    </>
  );
}

function CompanyDetail({ company }: { company: Company }) {
  const { data: people } = db.useQuery<Person>("Person", {
    where: { companyId: company.id },
  });
  return (
    <>
      <div className="detail-header">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            marginBottom: 10,
          }}
        >
          <div
            className="avatar avatar-lg"
            style={{ backgroundColor: "#fde68a" }}
          >
            {initials(company.name)}
          </div>
          <div>
            <div className="detail-name">{company.name}</div>
            <div className="detail-sub">
              {company.domain || "—"}
              {company.industry ? ` · ${company.industry}` : ""}
            </div>
          </div>
        </div>
      </div>
      <div className="detail-section">
        <div className="detail-section-title">Details</div>
        <div className="detail-kv">
          <div className="detail-kv-k">Status</div>
          <div className="detail-kv-v">
            <span className="pill pill-gray">{company.status}</span>
          </div>
          <div className="detail-kv-k">Size</div>
          <div className="detail-kv-v">{company.sizeBucket || "—"}</div>
          <div className="detail-kv-k">Domain</div>
          <div className="detail-kv-v">{company.domain || "—"}</div>
          <div className="detail-kv-k">Created</div>
          <div className="detail-kv-v">{ago(company.createdAt)}</div>
        </div>
      </div>
      <div className="detail-section">
        <div className="detail-section-title">
          People · {(people ?? []).length}
        </div>
        {(people ?? []).length === 0 ? (
          <div style={{ fontSize: 12, color: "var(--text-dim)" }}>
            No people linked yet.
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {(people ?? []).map((p) => (
              <div
                key={p.id}
                style={{ display: "flex", alignItems: "center", gap: 8 }}
              >
                <div
                  className="avatar avatar-xs"
                  style={{ backgroundColor: "#c7d2fe" }}
                >
                  {initials(fullName(p))}
                </div>
                <span style={{ fontSize: 12.5, fontWeight: 500 }}>
                  {fullName(p)}
                </span>
                {p.title && (
                  <span style={{ fontSize: 11, color: "var(--text-dim)" }}>
                    {p.title}
                  </span>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      <NotesSection targetType="Company" targetId={company.id} />
      <TimelineSection targetType="Company" targetId={company.id} />
    </>
  );
}

function AddCompanyModal({ onClose }: { onClose: () => void }) {
  const [form, setForm] = useState({
    name: "",
    domain: "",
    industry: "",
    sizeBucket: "",
    status: "lead",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
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
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Add company</div>
        <div className="modal-subtitle">Track a new account.</div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
            autoFocus
            className="input"
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder="Acme Corp"
          />
        </label>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Domain</span>
            <input
              className="input"
              value={form.domain}
              onChange={(e) => setForm({ ...form, domain: e.target.value })}
              placeholder="acme.com"
            />
          </label>
          <label className="field">
            <span className="field-label">Industry</span>
            <input
              className="input"
              value={form.industry}
              onChange={(e) => setForm({ ...form, industry: e.target.value })}
            />
          </label>
        </div>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Size</span>
            <select
              className="select"
              value={form.sizeBucket}
              onChange={(e) =>
                setForm({ ...form, sizeBucket: e.target.value })
              }
            >
              <option value="">—</option>
              <option value="1-10">1–10</option>
              <option value="11-50">11–50</option>
              <option value="51-200">51–200</option>
              <option value="201-500">201–500</option>
              <option value="500+">500+</option>
            </select>
          </label>
          <label className="field">
            <span className="field-label">Status</span>
            <select
              className="select"
              value={form.status}
              onChange={(e) => setForm({ ...form, status: e.target.value })}
            >
              <option value="lead">Lead</option>
              <option value="active">Active</option>
              <option value="churned">Churned</option>
            </select>
          </label>
        </div>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.name.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Add company"}
          </button>
        </div>
      </div>
    </div>
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
    <>
      <div className="list-pane">
        <div className="list-header">
          <div className="list-title">People</div>
          <div className="list-count">{list.length}</div>
          <div className="spacer" />
          <button
            className="btn btn-primary"
            onClick={() => setAddOpen(true)}
          >
            <IconPlus /> Add person
          </button>
        </div>
        {list.length === 0 ? (
          <div className="empty">
            <div className="empty-title">No people yet</div>
            <div className="empty-body">
              Add a contact to link to a company.
            </div>
            <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
              Add person
            </button>
          </div>
        ) : (
          <div style={{ overflowY: "auto", flex: 1 }}>
            <table className="table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Title</th>
                  <th>Company</th>
                  <th>Email</th>
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
                      className={p.id === selectedId ? "selected" : ""}
                      onClick={() => setSelectedId(p.id)}
                    >
                      <td>
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                          }}
                        >
                          <div
                            className="avatar avatar-sm"
                            style={{ backgroundColor: "#c7d2fe" }}
                          >
                            {initials(fullName(p))}
                          </div>
                          <span style={{ fontWeight: 500 }}>{fullName(p)}</span>
                        </div>
                      </td>
                      <td style={{ color: "var(--text-muted)" }}>
                        {p.title || "—"}
                      </td>
                      <td>
                        {company ? (
                          <span className="chip-company">
                            <div
                              className="avatar avatar-xs"
                              style={{ backgroundColor: "#fde68a" }}
                            >
                              {initials(company.name)}
                            </div>
                            {company.name}
                          </span>
                        ) : (
                          <span style={{ color: "var(--text-dim)" }}>—</span>
                        )}
                      </td>
                      <td style={{ color: "var(--text-muted)" }}>
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
      <aside className="detail-pane">
        {selected ? (
          <PersonDetail person={selected} companyById={companyById} />
        ) : (
          <div className="empty">
            <div className="empty-body">Pick a person to see details.</div>
          </div>
        )}
      </aside>
      {addOpen && (
        <AddPersonModal
          companies={companies ?? []}
          onClose={() => setAddOpen(false)}
        />
      )}
    </>
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
      <div className="detail-header">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            marginBottom: 10,
          }}
        >
          <div
            className="avatar avatar-lg"
            style={{ backgroundColor: "#c7d2fe" }}
          >
            {initials(fullName(person))}
          </div>
          <div>
            <div className="detail-name">{fullName(person)}</div>
            <div className="detail-sub">
              {person.title || "—"}
              {company ? ` · ${company.name}` : ""}
            </div>
          </div>
        </div>
      </div>
      <div className="detail-section">
        <div className="detail-section-title">Contact</div>
        <div className="detail-kv">
          <div className="detail-kv-k">Email</div>
          <div className="detail-kv-v">{person.email || "—"}</div>
          <div className="detail-kv-k">Phone</div>
          <div className="detail-kv-v">{person.phone || "—"}</div>
          <div className="detail-kv-k">Company</div>
          <div className="detail-kv-v">{company?.name || "—"}</div>
          <div className="detail-kv-k">Created</div>
          <div className="detail-kv-v">{ago(person.createdAt)}</div>
        </div>
      </div>
      <NotesSection targetType="Person" targetId={person.id} />
      <TimelineSection targetType="Person" targetId={person.id} />
    </>
  );
}

function AddPersonModal({
  companies,
  onClose,
}: {
  companies: Company[];
  onClose: () => void;
}) {
  const [form, setForm] = useState({
    firstName: "",
    lastName: "",
    email: "",
    phone: "",
    title: "",
    companyId: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
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
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">Add person</div>
        <div className="modal-subtitle">Contact in your network.</div>
        <div className="row-2">
          <label className="field">
            <span className="field-label">First name</span>
            <input
              autoFocus
              className="input"
              value={form.firstName}
              onChange={(e) =>
                setForm({ ...form, firstName: e.target.value })
              }
            />
          </label>
          <label className="field">
            <span className="field-label">Last name</span>
            <input
              className="input"
              value={form.lastName}
              onChange={(e) => setForm({ ...form, lastName: e.target.value })}
            />
          </label>
        </div>
        <label className="field">
          <span className="field-label">Title</span>
          <input
            className="input"
            value={form.title}
            onChange={(e) => setForm({ ...form, title: e.target.value })}
          />
        </label>
        <label className="field">
          <span className="field-label">Company</span>
          <select
            className="select"
            value={form.companyId}
            onChange={(e) => setForm({ ...form, companyId: e.target.value })}
          >
            <option value="">—</option>
            {companies.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
        </label>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Email</span>
            <input
              className="input"
              value={form.email}
              onChange={(e) => setForm({ ...form, email: e.target.value })}
            />
          </label>
          <label className="field">
            <span className="field-label">Phone</span>
            <input
              className="input"
              value={form.phone}
              onChange={(e) => setForm({ ...form, phone: e.target.value })}
            />
          </label>
        </div>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.firstName.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Add person"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Deals page — Kanban pipeline
// ---------------------------------------------------------------------------

function DealsPage({ org }: { org: Organization }) {
  const { data: deals } = db.useQuery<Deal>("Deal", {
    where: { orgId: org.id },
  });
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
    <div style={{ flex: 1, display: "flex", flexDirection: "column" }}>
      <div className="list-header">
        <div className="list-title">Pipeline</div>
        <div className="list-count">{(deals ?? []).length}</div>
        <div className="spacer" />
        <button className="btn btn-primary" onClick={() => setAddOpen(true)}>
          <IconPlus /> New deal
        </button>
      </div>
      <div className="kanban">
        {STAGES.map((stage) => {
          const items = byStage[stage] ?? [];
          const total = items.reduce((s, d) => s + d.amount, 0);
          return (
            <div key={stage} className="kanban-col">
              <div className="kanban-col-header">
                <span
                  className={"pill " + STAGE_COLORS[stage]}
                  style={{ padding: "1px 7px" }}
                >
                  <span className="pill-dot" />
                  {STAGE_LABELS[stage]}
                </span>
                <span className="kanban-col-count">{items.length}</span>
                <span className="kanban-col-total">{money(total)}</span>
              </div>
              {items.map((d) => (
                <DealCard
                  key={d.id}
                  deal={d}
                  company={
                    d.companyId ? companyById.get(d.companyId) : undefined
                  }
                  person={d.personId ? personById.get(d.personId) : undefined}
                  onClick={() => setSelectedId(d.id)}
                />
              ))}
            </div>
          );
        })}
      </div>
      {addOpen && (
        <AddDealModal
          companies={companies ?? []}
          people={people ?? []}
          onClose={() => setAddOpen(false)}
        />
      )}
      {selected && (
        <DealModal
          deal={selected}
          company={
            selected.companyId ? companyById.get(selected.companyId) : undefined
          }
          onClose={() => setSelectedId(null)}
        />
      )}
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
    <div className="kanban-card" onClick={onClick}>
      <div
        style={{
          display: "flex",
          alignItems: "flex-start",
          justifyContent: "space-between",
          gap: 8,
        }}
      >
        <div className="kanban-card-name">{deal.name}</div>
        <div className="kanban-card-amount">{money(deal.amount)}</div>
      </div>
      {(company || person) && (
        <div className="kanban-card-meta">
          {company && (
            <span className="chip-company">
              <div
                className="avatar avatar-xs"
                style={{ backgroundColor: "#fde68a" }}
              >
                {initials(company.name)}
              </div>
              {company.name}
            </span>
          )}
          {person && (
            <span style={{ color: "var(--text-muted)" }}>
              · {fullName(person)}
            </span>
          )}
        </div>
      )}
      <div
        className="kanban-card-meta"
        style={{ marginTop: 4, color: "var(--text-dim)" }}
      >
        {deal.probability}% · {ago(deal.createdAt)}
      </div>
    </div>
  );
}

function DealModal({
  deal,
  company,
  onClose,
}: {
  deal: Deal;
  company?: Company;
  onClose: () => void;
}) {
  async function moveTo(stage: string) {
    try {
      await callFn("updateDealStage", { dealId: deal.id, stage });
    } catch (e) {
      alert((e as Error).message);
    }
  }
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        onClick={(e) => e.stopPropagation()}
        style={{ width: 500 }}
      >
        <div className="modal-title">{deal.name}</div>
        <div className="modal-subtitle">
          {money(deal.amount)} · {company?.name || "No company"} ·{" "}
          {deal.probability}%
        </div>
        <div className="detail-section-title">Move to stage</div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {STAGES.map((s) => (
            <button
              key={s}
              onClick={() => {
                void moveTo(s);
              }}
              className={"pill " + STAGE_COLORS[s]}
              style={{
                padding: "4px 12px",
                border:
                  s === deal.stage
                    ? "2px solid var(--accent)"
                    : "1px solid transparent",
                fontSize: 12,
                cursor: "pointer",
              }}
            >
              <span className="pill-dot" />
              {STAGE_LABELS[s]}
            </button>
          ))}
        </div>
        <NotesSection targetType="Deal" targetId={deal.id} />
        <TimelineSection targetType="Deal" targetId={deal.id} />
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

function AddDealModal({
  companies,
  people,
  onClose,
}: {
  companies: Company[];
  people: Person[];
  onClose: () => void;
}) {
  const [form, setForm] = useState({
    name: "",
    companyId: "",
    personId: "",
    stage: "lead",
    amount: "",
  });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
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
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">New deal</div>
        <div className="modal-subtitle">Track an opportunity.</div>
        <label className="field">
          <span className="field-label">Name</span>
          <input
            autoFocus
            className="input"
            value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            placeholder="Acme — Q2 expansion"
          />
        </label>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Company</span>
            <select
              className="select"
              value={form.companyId}
              onChange={(e) => setForm({ ...form, companyId: e.target.value })}
            >
              <option value="">—</option>
              {companies.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span className="field-label">Person</span>
            <select
              className="select"
              value={form.personId}
              onChange={(e) => setForm({ ...form, personId: e.target.value })}
            >
              <option value="">—</option>
              {people.map((p) => (
                <option key={p.id} value={p.id}>
                  {fullName(p)}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="row-2">
          <label className="field">
            <span className="field-label">Stage</span>
            <select
              className="select"
              value={form.stage}
              onChange={(e) => setForm({ ...form, stage: e.target.value })}
            >
              {STAGES.map((s) => (
                <option key={s} value={s}>
                  {STAGE_LABELS[s]}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span className="field-label">Amount (USD)</span>
            <input
              className="input"
              type="number"
              min="0"
              step="100"
              value={form.amount}
              onChange={(e) => setForm({ ...form, amount: e.target.value })}
            />
          </label>
        </div>
        {err && <div className="error-text">{err}</div>}
        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            Cancel
          </button>
          <button
            className="btn btn-primary"
            disabled={busy || !form.name.trim()}
            onClick={() => void save()}
          >
            {busy ? "Saving…" : "Create deal"}
          </button>
        </div>
      </div>
    </div>
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
    <div className="detail-section">
      <div className="detail-section-title">Notes</div>
      <div style={{ display: "flex", gap: 6, marginBottom: 10 }}>
        <input
          className="input"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
          placeholder="Add a note…"
        />
        <button
          className="btn btn-primary"
          disabled={busy || !body.trim()}
          onClick={() => void add()}
        >
          Add
        </button>
      </div>
      {(notes ?? []).map((n) => (
        <NoteCard key={n.id} note={n} />
      ))}
    </div>
  );
}

function NoteCard({ note }: { note: Note }) {
  const { data: author } = db.useQueryOne<User>("User", note.authorId);
  return (
    <div className="note-card">
      <div className="note-body">{note.body}</div>
      <div className="note-meta">
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
    <div className="detail-section">
      <div className="detail-section-title">Activity</div>
      {(activities ?? []).length === 0 ? (
        <div style={{ fontSize: 12, color: "var(--text-dim)" }}>
          Nothing here yet.
        </div>
      ) : (
        (activities ?? []).map((a) => <ActivityRow key={a.id} activity={a} />)
      )}
    </div>
  );
}

function ActivityRow({ activity }: { activity: Activity }) {
  const { data: actor } = db.useQueryOne<User>("User", activity.actorId);
  let text = "";
  switch (activity.kind) {
    case "created":
      text = "created this record";
      break;
    case "stage_changed": {
      try {
        const meta = JSON.parse(activity.metaJson || "{}") as {
          from?: string;
          to?: string;
        };
        text = `moved stage from ${STAGE_LABELS[meta.from ?? ""] || meta.from} to ${STAGE_LABELS[meta.to ?? ""] || meta.to}`;
      } catch {
        text = "changed stage";
      }
      break;
    }
    case "note_added": {
      try {
        const meta = JSON.parse(activity.metaJson || "{}") as {
          preview?: string;
        };
        text = `added a note — "${meta.preview ?? ""}"`;
      } catch {
        text = "added a note";
      }
      break;
    }
    default:
      text = activity.kind;
  }
  return (
    <div className="timeline-item">
      <div className="timeline-icon">
        <IconDot />
      </div>
      <div className="timeline-body">
        <div className="timeline-text">
          <strong>{actor?.displayName ?? "Someone"}</strong> {text}
        </div>
        <div className="timeline-when">{ago(activity.createdAt)}</div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------

function BrandMark() {
  return (
    <div className="brand-mark">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
        <path
          d="M4 7l8-4 8 4v10l-8 4-8-4V7z"
          stroke="white"
          strokeWidth="2"
          strokeLinejoin="round"
        />
        <path
          d="M12 11v10M4 7l8 4 8-4"
          stroke="white"
          strokeWidth="2"
          strokeLinejoin="round"
        />
      </svg>
    </div>
  );
}
function IconBuilding() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <rect x="4" y="3" width="16" height="18" rx="2" stroke="currentColor" strokeWidth="2" />
      <path d="M9 7h2M9 11h2M9 15h2M13 7h2M13 11h2M13 15h2" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconUser() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="8" r="3.5" stroke="currentColor" strokeWidth="2" />
      <path d="M4 20c0-4 3.5-6 8-6s8 2 8 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
function IconMoney() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
      <path d="M12 3v18M16 6h-6a3 3 0 0 0 0 6h4a3 3 0 0 1 0 6H8" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
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
function IconDot() {
  return (
    <svg width="8" height="8" viewBox="0 0 10 10" fill="currentColor">
      <circle cx="5" cy="5" r="3" />
    </svg>
  );
}
