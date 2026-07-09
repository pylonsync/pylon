"use client";

import React, { useEffect, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import {
  createInvite,
  deleteOrg,
  listInvites,
  listOrgMembers,
  renameOrg,
  revokeInvite,
  useAuth,
  type OrgMember,
  type PendingInvite,
} from "@pylonsync/client";
import { Button } from "@/components/ui/button";
import {
  FolderKanban,
  Users2,
  CreditCard,
  ArrowRight,
  FolderPlus,
  Plus,
  Pencil,
  Archive,
  ArchiveRestore,
  Trash2,
  Check,
} from "lucide-react";

export interface Project {
  id: string;
  orgId: string;
  name: string;
  description?: string;
  status?: string;
  createdAt: string;
}

// OrgMember rows as returned by `serverData.list("OrgMember")` (the entity
// shape — camelCase fields). The read policy returns the caller's memberships
// across ALL their orgs plus everyone in the active org, so consumers must
// filter by `orgId === tenantId` to count just the active workspace.
export interface OrgMemberRow {
  id: string;
  orgId: string;
  userId: string;
  role: string;
}

// Every view receives its data from the SERVER (resolved via `serverData` +
// React 19 `use()` in the page) and the active org as `tenantId` (from
// `auth.tenant_id`). So the first client paint already has the right state —
// no `useAuth()`/fetch round-trip, no empty-state flash.

function NoOrg() {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <p className="text-sm text-zinc-500">
        You&apos;re not in a workspace yet. Each one is an isolated tenant — its
        projects and members are private to it.
      </p>
      <a
        href="/dashboard"
        className="mt-4 inline-flex h-9 items-center rounded-lg bg-zinc-900 px-4 text-[13px] font-medium text-white transition-colors hover:bg-zinc-700"
      >
        Set up your workspace
      </a>
      <p className="mt-3 text-xs text-zinc-400">
        …or pick one from the switcher in the sidebar.
      </p>
    </div>
  );
}

function Card({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-5">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-900">{title}</h2>
        {action}
      </div>
      <div className="mt-3">{children}</div>
    </div>
  );
}

// A stat tile: icon + label, a big value, and a small context line.
function StatCard({
  icon,
  label,
  value,
  hint,
}: {
  icon: React.ReactNode;
  label: string;
  value: React.ReactNode;
  hint?: React.ReactNode;
}) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="flex items-center gap-2 text-zinc-400">
        {icon}
        <span className="text-[11px] font-medium uppercase tracking-wide">
          {label}
        </span>
      </div>
      <div className="mt-2 text-2xl font-semibold text-zinc-900">{value}</div>
      {hint != null && <div className="mt-0.5 text-xs text-zinc-400">{hint}</div>}
    </div>
  );
}

// Deterministic, timezone-independent date (UTC parts) so the SSR render and
// client hydration always agree — a locale/tz-dependent format would mismatch
// and throw a React #418 hydration error.
const MONTHS = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
function fmtDate(iso: string) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}, ${d.getUTCFullYear()}`;
}

// A stable accent per project (hashed from its id) so each card/monogram gets a
// consistent color without persisting one.
const ACCENTS = [
  "bg-blue-50 text-blue-600",
  "bg-violet-50 text-violet-600",
  "bg-emerald-50 text-emerald-600",
  "bg-amber-50 text-amber-600",
  "bg-rose-50 text-rose-600",
  "bg-cyan-50 text-cyan-600",
];
function accentFor(id: string) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return ACCENTS[h % ACCENTS.length];
}
function monogram(name: string) {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const s =
    parts.length >= 2 ? parts[0][0] + parts[1][0] : name.trim()[0] ?? "?";
  return s.toUpperCase();
}

function StatusPill({ status }: { status: string }) {
  const archived = status === "archived";
  return (
    <span
      className={
        "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium " +
        (archived
          ? "bg-zinc-100 text-zinc-500"
          : "bg-emerald-50 text-emerald-600")
      }
    >
      <span
        className={
          "size-1.5 rounded-full " +
          (archived ? "bg-zinc-400" : "bg-emerald-500")
        }
      />
      {archived ? "Archived" : "Active"}
    </span>
  );
}

const inputCls =
  "h-9 w-full rounded-md border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10";

/* ============================ Overview ============================ */

export function Overview({
  tenantId,
  orgName,
  userEmail,
  projects,
  memberCount,
  plan,
}: {
  tenantId: string | null;
  orgName?: string;
  userEmail?: string;
  projects: Project[];
  memberCount: number;
  plan: string;
}) {
  if (!tenantId) return <NoOrg />;
  const activeCount = projects.filter(
    (p) => (p.status ?? "active") !== "archived",
  ).length;
  const archivedCount = projects.length - activeCount;
  const recent = [...projects]
    .sort((a, b) => (a.createdAt < b.createdAt ? 1 : -1))
    .slice(0, 5);
  const firstName = (userEmail ?? "").split("@")[0];

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-xl font-semibold tracking-tight text-zinc-900">
          {orgName ? `${orgName} overview` : "Overview"}
        </h1>
        <p className="mt-1 text-sm text-zinc-500">
          {firstName ? `Welcome back, ${firstName}. ` : ""}Here&apos;s what&apos;s
          happening in your workspace.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <StatCard
          icon={<FolderKanban className="size-4" />}
          label="Projects"
          value={activeCount}
          hint={archivedCount > 0 ? `${archivedCount} archived` : "active"}
        />
        <StatCard
          icon={<Users2 className="size-4" />}
          label="Members"
          value={memberCount}
          hint={memberCount === 1 ? "just you" : "in this workspace"}
        />
        <StatCard
          icon={<CreditCard className="size-4" />}
          label="Plan"
          value={<span className="capitalize">{plan}</span>}
          hint={
            plan === "free" ? (
              <a
                href="/dashboard/billing"
                className="text-brand hover:underline"
              >
                Upgrade to Pro →
              </a>
            ) : (
              "thanks for the support"
            )
          }
        />
      </div>

      <Card
        title="Recent projects"
        action={
          <a
            href="/dashboard/projects"
            className="inline-flex items-center gap-1 text-[13px] font-medium text-brand hover:underline"
          >
            View all <ArrowRight className="size-3.5" />
          </a>
        }
      >
        {recent.length === 0 ? (
          <div className="flex flex-col items-center gap-2 py-8 text-center">
            <span className="flex size-10 items-center justify-center rounded-xl bg-zinc-50 text-zinc-400">
              <FolderPlus className="size-5" />
            </span>
            <p className="text-sm text-zinc-500">No projects yet.</p>
            <a
              href="/dashboard/projects"
              className="text-[13px] font-medium text-brand hover:underline"
            >
              Create your first project →
            </a>
          </div>
        ) : (
          <ul className="-my-1 divide-y divide-zinc-100">
            {recent.map((p) => (
              <li key={p.id} className="flex items-center gap-3 py-2.5">
                <span
                  className={
                    "flex size-8 shrink-0 items-center justify-center rounded-lg text-[12px] font-semibold " +
                    accentFor(p.id)
                  }
                >
                  {monogram(p.name)}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-zinc-900">
                    {p.name}
                  </div>
                  <div className="truncate text-xs text-zinc-400">
                    Created {fmtDate(p.createdAt)}
                  </div>
                </div>
                <StatusPill status={p.status ?? "active"} />
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}

/* ============================ Projects ============================ */

export function Projects({
  tenantId,
  initial,
}: {
  tenantId: string | null;
  initial: Project[];
}) {
  if (!tenantId) return <NoOrg />;
  return <ProjectsList orgId={tenantId} initial={initial} />;
}

// Live + optimistic, seeded from the server. `db.useQuery` is reactive
// (db.insert/db.delete update it instantly across tabs), but until the first
// server-confirmed sync settles we render the server-passed `initial` rows — so
// there's no flash of an empty list on load. The policy gates reads on
// `auth.tenantId == data.orgId`, so this is only ever this org's projects.
function ProjectsList({
  orgId,
  initial,
}: {
  orgId: string;
  initial: Project[];
}) {
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  // Render the server-seeded `initial` rows on the server AND the client's
  // first paint, then swap to the live reactive query once mounted. Gating the
  // swap on a post-mount flag (rather than `loading`, which is already settled
  // during SSR) keeps the server HTML and the first client render identical —
  // no hydration mismatch, and still no empty-state flash.
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  const { data, loading } = db.useQuery<Project>("Project");
  const rows =
    !hydrated || loading ? initial : data.filter((p) => p.orgId === orgId);
  const projects = rows
    .slice()
    .sort((a, b) => {
      // Active first, then newest first.
      const aa = (a.status ?? "active") === "archived" ? 1 : 0;
      const bb = (b.status ?? "active") === "archived" ? 1 : 0;
      if (aa !== bb) return aa - bb;
      return a.createdAt < b.createdAt ? 1 : -1;
    });

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const value = name.trim();
    if (!value) return;
    const description = desc.trim();
    setName("");
    setDesc("");
    await db.insert("Project", {
      orgId,
      name: value,
      status: "active",
      ...(description ? { description } : {}),
    });
  }

  return (
    <div className="space-y-5">
      <div className="rounded-xl border border-zinc-200 bg-white p-4">
        <form
          onSubmit={add}
          className="flex flex-col gap-2 sm:flex-row sm:items-center"
        >
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Project name"
            aria-label="Project name"
            className={inputCls + " sm:flex-1"}
          />
          <input
            value={desc}
            onChange={(e) => setDesc(e.target.value)}
            placeholder="Short description (optional)"
            aria-label="Project description"
            className={inputCls + " sm:flex-1"}
          />
          <Button type="submit" size="sm" className="shrink-0">
            <Plus className="size-4" /> New project
          </Button>
        </form>
        <p className="mt-2 text-xs text-zinc-400">
          Tenant-scoped + live — only this workspace&apos;s projects (enforced by
          policy), and every change syncs across tabs instantly.
        </p>
      </div>

      {projects.length === 0 ? (
        <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-zinc-300 py-14 text-center">
          <span className="flex size-11 items-center justify-center rounded-xl bg-zinc-50 text-zinc-400">
            <FolderPlus className="size-5" />
          </span>
          <p className="text-sm font-medium text-zinc-700">No projects yet</p>
          <p className="text-xs text-zinc-400">
            Create your first project above to get started.
          </p>
        </div>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {projects.map((p) => (
            <ProjectCard key={p.id} p={p} />
          ))}
        </div>
      )}
    </div>
  );
}

function IconBtn({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={
        "flex size-7 items-center justify-center rounded-md text-zinc-400 transition-colors hover:bg-zinc-100 " +
        (danger ? "hover:text-red-600" : "hover:text-zinc-700")
      }
    >
      {children}
    </button>
  );
}

// One project card. Reads/writes live via the reactive `db` — rename, archive,
// and delete are optimistic and sync across tabs (all gated by the tenant
// policy). Editing swaps the card for an inline name + description form.
function ProjectCard({ p }: { p: Project }) {
  const archived = (p.status ?? "active") === "archived";
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(p.name);
  const [desc, setDesc] = useState(p.description ?? "");

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const n = name.trim();
    if (!n) return;
    setEditing(false);
    await db.update("Project", p.id, {
      name: n,
      description: desc.trim() || undefined,
    });
  }
  function cancel() {
    setEditing(false);
    setName(p.name);
    setDesc(p.description ?? "");
  }

  if (editing) {
    return (
      <form
        onSubmit={save}
        className="flex flex-col gap-2 rounded-xl border border-zinc-300 bg-white p-4"
      >
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          aria-label="Project name"
          className={inputCls}
        />
        <textarea
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
          rows={2}
          placeholder="Add a description…"
          aria-label="Project description"
          className={inputCls + " h-auto resize-none py-2"}
        />
        <div className="flex items-center gap-2">
          <Button type="submit" size="sm">
            <Check className="size-3.5" /> Save
          </Button>
          <button
            type="button"
            onClick={cancel}
            className="text-[13px] font-medium text-zinc-500 transition-colors hover:text-zinc-800"
          >
            Cancel
          </button>
        </div>
      </form>
    );
  }

  return (
    <div
      className={
        "flex flex-col rounded-xl border border-zinc-200 bg-white p-4 transition-colors hover:border-zinc-300 " +
        (archived ? "opacity-60" : "")
      }
    >
      <div className="flex items-start gap-3">
        <span
          className={
            "flex size-9 shrink-0 items-center justify-center rounded-lg text-sm font-semibold " +
            accentFor(p.id)
          }
        >
          {monogram(p.name)}
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-zinc-900">
            {p.name}
          </div>
          <p
            className={
              "mt-0.5 line-clamp-2 text-xs " +
              (p.description ? "text-zinc-500" : "text-zinc-300")
            }
          >
            {p.description || "No description"}
          </p>
        </div>
        <div className="-mr-1 flex shrink-0 items-center">
          <IconBtn label="Edit project" onClick={() => setEditing(true)}>
            <Pencil className="size-3.5" />
          </IconBtn>
          <IconBtn
            label={archived ? "Unarchive project" : "Archive project"}
            onClick={() =>
              db.update("Project", p.id, {
                status: archived ? "active" : "archived",
              })
            }
          >
            {archived ? (
              <ArchiveRestore className="size-3.5" />
            ) : (
              <Archive className="size-3.5" />
            )}
          </IconBtn>
          <IconBtn
            label="Delete project"
            danger
            onClick={() => db.delete("Project", p.id)}
          >
            <Trash2 className="size-3.5" />
          </IconBtn>
        </div>
      </div>
      <div className="mt-4 flex items-center justify-between">
        <StatusPill status={p.status ?? "active"} />
        <span className="text-[11px] text-zinc-400">
          Created {fmtDate(p.createdAt)}
        </span>
      </div>
    </div>
  );
}

/* ============================= Members ============================ */

// Active-org info passed from the server (resolved via serverData) so the views
// render real names instead of raw ids — and instantly, with no fetch flash.
export interface OrgInfo {
  id: string;
  name: string;
  createdAt: string;
}

const isManager = (role: string) => role === "owner" || role === "admin";

function RoleBadge({ role }: { role: string }) {
  const tone =
    role === "owner"
      ? "bg-brand-soft text-brand"
      : role === "admin"
        ? "bg-amber-50 text-amber-700"
        : "bg-zinc-100 text-zinc-600";
  return (
    <span
      className={
        "rounded-full px-2 py-0.5 text-[11px] font-medium capitalize " + tone
      }
    >
      {role}
    </span>
  );
}

export function Members({
  tenantId,
  currentUserId,
  role,
}: {
  tenantId: string | null;
  currentUserId: string | null;
  role: string;
}) {
  if (!tenantId) return <NoOrg />;
  return (
    <MembersList orgId={tenantId} currentUserId={currentUserId} role={role} />
  );
}

// The roster comes from the framework's members endpoint, which joins each
// member's email + name server-side (the User read policy blocks reading other
// users via sync, so this trusted endpoint is the only place to get identities).
// Invites + the roster are gated to owners/admins here AND on the server.
function MembersList({
  orgId,
  currentUserId,
  role,
}: {
  orgId: string;
  currentUserId: string | null;
  role: string;
}) {
  const canManage = isManager(role);
  const [members, setMembers] = useState<OrgMember[] | null>(null);
  const [invites, setInvites] = useState<PendingInvite[] | null>(null);
  const [email, setEmail] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [inviting, setInviting] = useState(false);

  async function load() {
    setMembers(await listOrgMembers(orgId));
    // Pending invites are admin-gated server-side; only fetch when we'd be
    // allowed to see them (avoids a guaranteed 403 for plain members).
    if (canManage) setInvites(await listInvites(orgId));
  }
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [orgId]);

  async function invite(e: React.FormEvent) {
    e.preventDefault();
    const value = email.trim();
    if (!value) return;
    setInviting(true);
    setNote(null);
    try {
      await createInvite(orgId, value, "member");
      setEmail("");
      setNote(`Invite sent to ${value}.`);
      void load();
    } catch {
      setNote("Couldn't send that invite — check the address and your role.");
    } finally {
      setInviting(false);
    }
  }

  async function revoke(inviteId: string) {
    setInvites((prev) => prev?.filter((i) => i.id !== inviteId) ?? null);
    try {
      await revokeInvite(orgId, inviteId);
    } finally {
      void load();
    }
  }

  return (
    <div className="space-y-6">
    <Card title="Members" action={members ? <Count n={members.length} /> : null}>
      {canManage && (
        <form onSubmit={invite} className="flex items-center gap-2">
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="teammate@company.com"
            aria-label="Invite email"
            className={inputCls}
          />
          <Button type="submit" size="sm" disabled={inviting || !email.trim()}>
            {inviting ? "…" : "Invite"}
          </Button>
        </form>
      )}
      {note && <p className="mt-2 text-xs text-zinc-500">{note}</p>}

      <ul className="mt-3 divide-y divide-zinc-100">
        {members === null
          ? // Skeleton rows while the roster loads — sized to the real row so
            // there's no layout shift when it lands.
            Array.from({ length: 3 }).map((_, i) => (
              <li key={i} className="flex items-center gap-3 py-2.5">
                <div className="size-8 animate-pulse rounded-full bg-zinc-100" />
                <div className="h-3 w-40 animate-pulse rounded bg-zinc-100" />
              </li>
            ))
          : members.map((m) => {
              const label = m.name || m.email || "Unknown member";
              const initial = (label.trim()[0] || "?").toUpperCase();
              const isMe = m.user_id === currentUserId;
              return (
                <li
                  key={m.user_id}
                  className="flex items-center gap-3 py-2.5"
                >
                  <span className="flex size-8 shrink-0 items-center justify-center rounded-full bg-zinc-100 text-[12px] font-semibold text-zinc-600">
                    {initial}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium text-zinc-900">
                        {m.name || m.email || "Unknown member"}
                      </span>
                      {isMe && (
                        <span className="rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500">
                          You
                        </span>
                      )}
                    </div>
                    {m.name && m.email && m.name !== m.email && (
                      <div className="truncate text-xs text-zinc-500">
                        {m.email}
                      </div>
                    )}
                  </div>
                  <RoleBadge role={m.role} />
                </li>
              );
            })}
      </ul>
      {!canManage && (
        <p className="mt-3 text-xs text-zinc-400">
          Only owners and admins can invite new members.
        </p>
      )}
    </Card>

    {canManage && invites && invites.length > 0 && (
      <Card
        title="Pending invitations"
        action={
          <span className="text-[13px] text-zinc-400">
            {invites.length} pending
          </span>
        }
      >
        <ul className="divide-y divide-zinc-100">
          {invites.map((inv) => (
            <li key={inv.id} className="flex items-center gap-3 py-2.5">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-full border border-dashed border-zinc-300 text-zinc-400">
                <MailIcon />
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-sm font-medium text-zinc-900">
                  {inv.email}
                </div>
                <div className="text-xs text-zinc-500">
                  Invited · expires {formatDate(unixToIso(inv.expires_at))}
                </div>
              </div>
              <RoleBadge role={inv.role} />
              <button
                type="button"
                onClick={() => revoke(inv.id)}
                className="text-[13px] font-medium text-zinc-400 transition-colors hover:text-red-600"
              >
                Revoke
              </button>
            </li>
          ))}
        </ul>
      </Card>
    )}
    </div>
  );
}

function MailIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m3 7 9 6 9-6" />
    </svg>
  );
}

// PendingInvite.expires_at is unix SECONDS; formatDate() wants an ISO string.
function unixToIso(sec: number) {
  return new Date(sec * 1000).toISOString();
}

function Count({ n }: { n: number }) {
  return (
    <span className="text-[13px] text-zinc-400">
      {n} {n === 1 ? "person" : "people"}
    </span>
  );
}

/* ============================ Settings ============================ */

export function Settings({
  org,
  role,
  memberCount,
}: {
  org: OrgInfo | null;
  role: string;
  memberCount: number;
}) {
  if (!org) return <NoOrg />;
  return <SettingsView org={org} role={role} memberCount={memberCount} />;
}

function SettingsView({
  org,
  role,
  memberCount,
}: {
  org: OrgInfo;
  role: string;
  memberCount: number;
}) {
  const { clearOrg } = useAuth();
  const [name, setName] = useState(org.name);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canManage = isManager(role);
  const canDelete = role === "owner";
  const dirty = name.trim() !== org.name && name.trim().length > 0;

  async function rename(e: React.FormEvent) {
    e.preventDefault();
    if (!dirty) return;
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      await renameOrg(org.id, name.trim());
      setSaved(true);
      // Reflect the new name in the sidebar switcher + everywhere else.
      window.location.reload();
    } catch {
      setError("Couldn't rename — only owners and admins can.");
      setSaving(false);
    }
  }

  return (
    <div className="max-w-2xl space-y-6">
      <Card title="Workspace">
        <form onSubmit={rename} className="space-y-3">
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">
              Name
            </span>
            <div className="flex items-center gap-2">
              <input
                value={name}
                onChange={(e) => {
                  setName(e.target.value);
                  setSaved(false);
                }}
                disabled={!canManage}
                aria-label="Workspace name"
                className={inputCls + " disabled:bg-zinc-50 disabled:text-zinc-500"}
              />
              {canManage && (
                <Button type="submit" size="sm" disabled={!dirty || saving}>
                  {saving ? "…" : "Save"}
                </Button>
              )}
            </div>
          </label>
          {saved && (
            <p className="text-xs text-green-600">Workspace name updated.</p>
          )}
          {error && <p className="text-xs text-red-600">{error}</p>}
        </form>

        <dl className="mt-5 grid grid-cols-2 gap-y-3 border-t border-zinc-100 pt-4 text-sm">
          <dt className="text-zinc-500">Your role</dt>
          <dd className="text-right">
            <RoleBadge role={role} />
          </dd>
          <dt className="text-zinc-500">Members</dt>
          <dd className="text-right text-zinc-900">{memberCount}</dd>
          <dt className="text-zinc-500">Created</dt>
          <dd className="text-right text-zinc-900">{formatDate(org.createdAt)}</dd>
        </dl>
      </Card>

      <Card title="Danger zone">
        {canDelete ? (
          <DeleteOrg org={org} onDeleted={clearOrg} />
        ) : (
          <p className="text-sm text-zinc-500">
            Only the workspace owner can delete this workspace.
          </p>
        )}
      </Card>
    </div>
  );
}

// Real, irreversible delete: type the workspace name to confirm, then call the
// framework's owner-gated DELETE endpoint, drop the active org, and bounce to
// the dashboard — which selects another workspace or provisions a fresh one.
function DeleteOrg({
  org,
  onDeleted,
}: {
  org: OrgInfo;
  onDeleted: () => Promise<void>;
}) {
  const [confirm, setConfirm] = useState("");
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const armed = confirm.trim() === org.name;

  async function remove() {
    if (!armed) return;
    setDeleting(true);
    setError(null);
    try {
      await deleteOrg(org.id);
      await onDeleted();
      window.location.assign("/dashboard");
    } catch {
      setError("Delete failed. Try again.");
      setDeleting(false);
    }
  }

  return (
    <div className="space-y-3">
      <p className="text-sm text-zinc-600">
        Deleting <span className="font-medium">{org.name}</span> removes its
        projects and members for everyone. This can&apos;t be undone.
      </p>
      <label className="block">
        <span className="mb-1.5 block text-[13px] text-zinc-500">
          Type <span className="font-medium text-zinc-700">{org.name}</span> to
          confirm
        </span>
        <input
          value={confirm}
          onChange={(e) => setConfirm(e.target.value)}
          aria-label="Confirm workspace name"
          className={inputCls}
        />
      </label>
      {error && <p className="text-xs text-red-600">{error}</p>}
      <button
        type="button"
        onClick={remove}
        disabled={!armed || deleting}
        className="inline-flex h-9 items-center rounded-lg bg-red-600 px-4 text-[13px] font-medium text-white transition-colors hover:bg-red-700 disabled:opacity-40"
      >
        {deleting ? "Deleting…" : "Delete workspace"}
      </button>
    </div>
  );
}

function formatDate(iso: string) {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "—";
  return new Date(t).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

/* ============================ Billing ============================ */

// One row of the @pylonsync/stripe StripeSubscription entity (client-readable
// via the plugin's policy, scoped to the active org by referenceId).
export interface Subscription {
  id: string;
  referenceId: string;
  plan: string;
  status: string;
  cancelAtPeriodEnd?: boolean;
  currentPeriodEnd?: string;
}

const ACTIVE = ["active", "trialing", "past_due"];

export function Billing({
  tenantId,
  role,
  subscription,
}: {
  tenantId: string | null;
  role: string;
  subscription: Subscription | null;
}) {
  if (!tenantId) return <NoOrg />;
  return (
    <BillingView tenantId={tenantId} role={role} subscription={subscription} />
  );
}

// Real Stripe billing via the plugin's actions (callFn → /api/fn/*). Upgrade
// opens Stripe Checkout; Manage opens the Customer Portal; the webhook keeps the
// StripeSubscription row in sync, so a full page load after returning reflects
// the new plan. Until STRIPE_SECRET_KEY + STRIPE_PRICE_PRO are set the actions
// return STRIPE_NOT_CONFIGURED, which we surface as a "connect Stripe" state.
function BillingView({
  tenantId,
  role,
  subscription,
}: {
  tenantId: string;
  role: string;
  subscription: Subscription | null;
}) {
  const canManage = isManager(role);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const active = subscription && ACTIVE.includes(subscription.status);
  const planLabel = active ? subscription!.plan : "free";

  function origin() {
    return typeof window !== "undefined" ? window.location.origin : "";
  }

  async function run(action: string, fn: () => Promise<void>) {
    setBusy(action);
    setError(null);
    try {
      await fn();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // A missing STRIPE_PRICE_PRO surfaces as "plan pro has no monthly
      // priceId" rather than STRIPE_NOT_CONFIGURED, so treat the price/config
      // errors the same — both mean "finish the Stripe setup".
      setError(
        /not.?configured|STRIPE_NOT_CONFIGURED|no monthly price|priceId/i.test(
          msg,
        )
          ? "Stripe isn't configured yet — set STRIPE_SECRET_KEY + STRIPE_PRICE_PRO to enable billing."
          : msg,
      );
      setBusy(null);
    }
  }

  async function upgrade() {
    await run("upgrade", async () => {
      const res = await callFn<{ url: string }>("createCheckoutSession", {
        plan: "pro",
        referenceId: tenantId,
        successUrl: `${origin()}/dashboard/billing`,
        cancelUrl: `${origin()}/dashboard/billing`,
      });
      window.location.assign(res.url);
    });
  }

  async function manage() {
    await run("manage", async () => {
      const res = await callFn<{ url: string }>("createBillingPortalSession", {
        referenceId: tenantId,
        returnUrl: `${origin()}/dashboard/billing`,
      });
      window.location.assign(res.url);
    });
  }

  async function cancel() {
    await run("cancel", async () => {
      await callFn("cancelSubscription", {
        referenceId: tenantId,
        scheduleAtPeriodEnd: true,
      });
      window.location.reload();
    });
  }

  async function restore() {
    await run("restore", async () => {
      await callFn("restoreSubscription", { referenceId: tenantId });
      window.location.reload();
    });
  }

  return (
    <div className="max-w-2xl space-y-6">
      <Card title="Plan">
        <div className="flex items-center justify-between">
          <div>
            <div className="flex items-center gap-2">
              <span className="text-2xl font-semibold capitalize text-zinc-900">
                {planLabel}
              </span>
              {active && <RoleBadge role={subscription!.status} />}
            </div>
            {active && subscription!.currentPeriodEnd && (
              <p className="mt-1 text-[13px] text-zinc-500">
                {subscription!.cancelAtPeriodEnd ? "Ends" : "Renews"}{" "}
                {formatDate(subscription!.currentPeriodEnd)}
              </p>
            )}
            {!active && (
              <p className="mt-1 text-[13px] text-zinc-500">
                The free plan. Upgrade to Pro for higher limits.
              </p>
            )}
          </div>

          {canManage ? (
            active ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={manage}
                disabled={!!busy}
              >
                {busy === "manage" ? "…" : "Manage billing"}
              </Button>
            ) : (
              <Button type="button" onClick={upgrade} disabled={!!busy}>
                {busy === "upgrade" ? "…" : "Upgrade to Pro"}
              </Button>
            )
          ) : (
            <span className="text-xs text-zinc-400">
              Owners and admins manage billing.
            </span>
          )}
        </div>

        {active && canManage && (
          <div className="mt-4 border-t border-zinc-100 pt-3 text-[13px]">
            {subscription!.cancelAtPeriodEnd ? (
              <button
                type="button"
                onClick={restore}
                disabled={!!busy}
                className="font-medium text-brand hover:underline disabled:opacity-50"
              >
                {busy === "restore" ? "…" : "Resume subscription"}
              </button>
            ) : (
              <button
                type="button"
                onClick={cancel}
                disabled={!!busy}
                className="text-zinc-500 hover:text-red-600 disabled:opacity-50"
              >
                {busy === "cancel" ? "…" : "Cancel at period end"}
              </button>
            )}
          </div>
        )}

        {error && <p className="mt-3 text-xs text-red-600">{error}</p>}
      </Card>

      <p className="text-xs text-zinc-400">
        Powered by Stripe Checkout + Customer Portal via the @pylonsync/stripe
        plugin. Set STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET, and STRIPE_PRICE_PRO
        to go live; point the webhook at <code>/api/fn/stripeWebhook</code>.
      </p>
    </div>
  );
}
