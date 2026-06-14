"use client";

import React, { useCallback, useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import {
  useAuth,
  listOrgMembers,
  createInvite,
  type OrgMember,
} from "@pylonsync/client";
import { Button } from "@/components/ui/button";

export interface Project {
  id: string;
  orgId: string;
  name: string;
  createdAt: string;
}

// Shown when no org is selected — the sidebar's <OrganizationSwitcher> is where
// you pick or create one. Every dashboard view keys off the active tenant.
function NoOrg() {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center text-sm text-zinc-500">
      Select or create an organization from the sidebar to get started. Each org
      is an isolated tenant — its projects and members are private to it.
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

export function Overview() {
  const { tenantId } = useAuth();
  if (!tenantId) return <NoOrg />;
  return <OverviewInner orgId={tenantId} />;
}

function OverviewInner({ orgId }: { orgId: string }) {
  const { data: all } = db.useQuery<Project>("Project");
  const projects = all.filter((p) => p.orgId === orgId);
  const [memberCount, setMemberCount] = useState<number | null>(null);
  useEffect(() => {
    void listOrgMembers(orgId).then((m) => setMemberCount(m.length));
  }, [orgId]);

  return (
    <div className="space-y-6">
      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Projects" value={projects.length} />
        <Stat label="Members" value={memberCount ?? "—"} />
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

export function Projects() {
  const { tenantId } = useAuth();
  if (!tenantId) return <NoOrg />;
  return <ProjectsList orgId={tenantId} />;
}

// Tenant-scoped data. `db.useQuery("Project")` returns only your active org's
// projects (the policy gates on `auth.tenantId == data.orgId`), and switching
// orgs re-syncs the list. `db.insert` is optimistic; we pass `orgId` = the
// active tenant so the row lands in this org — the policy rejects any other.
function ProjectsList({ orgId }: { orgId: string }) {
  const [name, setName] = useState("");
  const { data: all } = db.useQuery<Project>("Project");
  const projects = all.filter((p) => p.orgId === orgId);

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
        Tenant-scoped: only this org&apos;s projects, enforced by policy — switch
        orgs and the list changes.
      </p>
    </Card>
  );
}

/* ============================= Members ============================ */

export function Members() {
  const { tenantId } = useAuth();
  if (!tenantId) return <NoOrg />;
  return <MembersList orgId={tenantId} />;
}

// Membership + invites go through the framework's /api/auth/orgs/:id endpoints
// (the @pylonsync/client helpers). The framework gates invites to org admins,
// so a member calling createInvite gets a 403 — real RBAC, no extra code.
function MembersList({ orgId }: { orgId: string }) {
  const [members, setMembers] = useState<OrgMember[] | null>(null);
  const [email, setEmail] = useState("");
  const [note, setNote] = useState("");

  const refresh = useCallback(() => {
    void listOrgMembers(orgId).then(setMembers);
  }, [orgId]);

  useEffect(() => {
    setMembers(null);
    refresh();
  }, [refresh]);

  async function invite(e: React.FormEvent) {
    e.preventDefault();
    const value = email.trim();
    if (!value) return;
    setEmail("");
    setNote("");
    try {
      await createInvite(orgId, value, "member");
      setNote(`Invited ${value}.`);
      refresh();
    } catch {
      setNote("Only org admins can invite members.");
    }
  }

  return (
    <Card title="Members">
      <form onSubmit={invite} className="flex items-center gap-2">
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="invite by email…"
          aria-label="Invite email"
          className={inputCls}
        />
        <Button type="submit" size="sm" variant="outline">
          Invite
        </Button>
      </form>
      {note && <p className="mt-2 text-xs text-zinc-500">{note}</p>}
      {members === null ? (
        <p className="mt-3 text-sm text-zinc-500">Loading…</p>
      ) : (
        <ul className="mt-3 space-y-1.5">
          {members.map((m) => (
            <li
              key={m.user_id}
              className="flex items-center justify-between rounded-md border border-zinc-200 px-3 py-2 text-sm"
            >
              <span className="truncate font-mono text-xs text-zinc-600">
                {shortId(m.user_id)}
              </span>
              <span className="text-xs text-zinc-500">{m.role}</span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}

/* ============================ Settings ============================ */

export function Settings() {
  const { tenantId } = useAuth();
  return (
    <div className="max-w-2xl space-y-6">
      <Card title="Workspace">
        <p className="text-sm text-zinc-600">
          Active organization:{" "}
          <span className="font-mono text-xs text-zinc-500">
            {tenantId ?? "none selected"}
          </span>
        </p>
        <p className="mt-2 text-sm text-zinc-500">
          Use the switcher in the sidebar to rename, switch between, or create an
          organization.
        </p>
      </Card>
      <Card title="Danger zone">
        <p className="text-sm text-zinc-500">
          Deleting an organization removes its projects and members for everyone.
          [Placeholder — wire up a delete action when you build it out.]
        </p>
      </Card>
    </div>
  );
}

function shortId(id: string) {
  return id.replace(/^user_/, "").slice(0, 10);
}
