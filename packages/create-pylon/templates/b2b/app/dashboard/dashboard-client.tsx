"use client";

import React, { useCallback, useEffect, useState } from "react";
import { db } from "@pylonsync/react";
import {
  useAuth,
  OrganizationSwitcher,
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

// The workspace. `<OrganizationSwitcher>` (from @pylonsync/client) lists your
// orgs, creates new ones, and switches your active tenant via
// /api/auth/select-org — all against the framework's built-in org system. The
// rest of the page keys off `tenantId` (your active org).
export function Workspace() {
  const { tenantId, signOut } = useAuth();

  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-3">
        <OrganizationSwitcher />
        <Button variant="ghost" size="sm" onClick={onSignOut}>
          Sign out
        </Button>
      </div>

      {tenantId ? (
        <div className="grid gap-6 sm:grid-cols-2">
          <Projects orgId={tenantId} />
          <Members orgId={tenantId} />
        </div>
      ) : (
        <div className="rounded-lg border border-dashed px-6 py-10 text-center text-sm text-muted-foreground">
          Create or select an organization above to get started. Each org is an
          isolated tenant — its projects and members are private to it.
        </div>
      )}
    </div>
  );
}

// Tenant-scoped data. `db.useQuery("Project")` returns only your active org's
// projects (the policy gates on `auth.tenantId == data.orgId`), and switching
// orgs re-syncs the list. `db.insert` is optimistic; we pass `orgId` = the
// active tenant so the row lands in this org — the policy rejects any other.
function Projects({ orgId }: { orgId: string }) {
  const [name, setName] = useState("");
  const { data: all } = db.useQuery<Project>("Project");
  // Defensive filter while a tenant switch re-syncs the local replica.
  const projects = all.filter((p) => p.orgId === orgId);

  async function add(e: React.FormEvent) {
    e.preventDefault();
    const value = name.trim();
    if (!value) return;
    setName("");
    await db.insert("Project", { orgId, name: value });
  }

  return (
    <section className="space-y-3">
      <h2 className="text-sm font-semibold">Projects</h2>
      <form onSubmit={add} className="flex items-center gap-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="New project…"
          aria-label="Project name"
          className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit" size="sm">
          Add
        </Button>
      </form>
      {projects.length === 0 ? (
        <p className="text-sm text-muted-foreground">No projects yet.</p>
      ) : (
        <ul className="space-y-1.5">
          {projects.map((p) => (
            <li
              key={p.id}
              className="flex items-center justify-between rounded-md border px-3 py-2 text-sm"
            >
              <span className="truncate">{p.name}</span>
              <button
                type="button"
                aria-label="Delete project"
                onClick={() => db.delete("Project", p.id)}
                className="text-muted-foreground/40 transition-colors hover:text-red-600"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="text-xs text-muted-foreground">
        Tenant-scoped: only this org&apos;s projects, enforced by policy — switch
        orgs and the list changes.
      </p>
    </section>
  );
}

// Membership + invites go through the framework's /api/auth/orgs/:id endpoints
// (the @pylonsync/client helpers). The framework gates invites to org admins,
// so a member calling createInvite gets a 403 — real RBAC, no extra code.
function Members({ orgId }: { orgId: string }) {
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
    <section className="space-y-3">
      <h2 className="text-sm font-semibold">Members</h2>
      {members === null ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : (
        <ul className="space-y-1.5">
          {members.map((m) => (
            <li
              key={m.user_id}
              className="flex items-center justify-between rounded-md border px-3 py-2 text-sm"
            >
              <span className="truncate font-mono text-xs">
                {shortId(m.user_id)}
              </span>
              <span className="text-xs text-muted-foreground">{m.role}</span>
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={invite} className="flex items-center gap-2">
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="invite by email…"
          aria-label="Invite email"
          className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm outline-none focus-visible:ring-1 focus-visible:ring-ring"
        />
        <Button type="submit" size="sm" variant="outline">
          Invite
        </Button>
      </form>
      {note && <p className="text-xs text-muted-foreground">{note}</p>}
    </section>
  );
}

function shortId(id: string) {
  return id.replace(/^user_/, "").slice(0, 10);
}
