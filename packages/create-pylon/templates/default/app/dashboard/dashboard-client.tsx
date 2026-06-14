"use client";

import React, { useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import {
  createInvite,
  deleteOrg,
  listOrgMembers,
  renameOrg,
  useAuth,
  type OrgMember,
} from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Project {
  id: string;
  orgId: string;
  name: string;
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
        href="/onboarding"
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

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">
        {label}
      </div>
      <div className="mt-1 text-2xl font-semibold text-zinc-900">{value}</div>
    </div>
  );
}

const inputCls =
  "h-9 w-full rounded-md border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900/10";

/* ============================ Overview ============================ */

export function Overview({
  tenantId,
  projects,
  memberCount,
}: {
  tenantId: string | null;
  projects: Project[];
  memberCount: number;
}) {
  if (!tenantId) return <NoOrg />;
  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Projects" value={projects.length} />
        <Stat label="Members" value={memberCount} />
        <Stat label="Plan" value="Free" />
      </div>
      <Card
        title="Recent projects"
        action={
          <a
            href="/dashboard/projects"
            className="text-[13px] font-medium text-brand hover:underline"
          >
            View all →
          </a>
        }
      >
        {projects.length === 0 ? (
          <p className="text-sm text-zinc-500">
            No projects yet — create one on the Projects tab.
          </p>
        ) : (
          <ul className="divide-y divide-zinc-100">
            {projects.slice(0, 5).map((p) => (
              <li key={p.id} className="py-2.5 text-sm text-zinc-700">
                {p.name}
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
  const { data, loading } = db.useQuery<Project>("Project");
  const projects = loading ? initial : data.filter((p) => p.orgId === orgId);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const value = name.trim();
    if (!value) return;
    setName("");
    await db.insert("Project", { orgId, name: value });
  }

  return (
    <Card title="Projects">
      <form onSubmit={add} className="flex items-center gap-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="New project…"
          aria-label="Project name"
          className={inputCls}
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      {projects.length === 0 ? (
        <p className="mt-3 text-sm text-zinc-500">No projects yet.</p>
      ) : (
        <ul className="mt-3 space-y-1.5">
          {projects.map((p) => (
            <li
              key={p.id}
              className="flex items-center justify-between rounded-md border border-zinc-200 px-3 py-2 text-sm"
            >
              <span className="truncate">{p.name}</span>
              <button
                type="button"
                aria-label="Delete project"
                onClick={() => db.delete("Project", p.id)}
                className="text-zinc-300 transition-colors hover:text-red-600"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="mt-3 text-xs text-zinc-400">
        Tenant-scoped + live: only this org&apos;s projects (enforced by policy),
        seeded from the server so there&apos;s no load flash.
      </p>
    </Card>
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
  const [email, setEmail] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [inviting, setInviting] = useState(false);

  async function load() {
    setMembers(await listOrgMembers(orgId));
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

  return (
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
                        {m.name || m.email || m.user_id}
                      </span>
                      {isMe && (
                        <span className="rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500">
                          You
                        </span>
                      )}
                    </div>
                    {m.name && m.email && (
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
  );
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
// framework's owner-gated DELETE endpoint, drop the active org, and bounce back
// to onboarding.
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
      window.location.assign("/onboarding");
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
