"use client";

import React, { useEffect, useState } from "react";
import { callFn } from "@pylonsync/react";
import {
  createInvite,
  deleteOrg,
  listInvites,
  listOrgMembers,
  removeMember,
  renameOrg,
  revokeInvite,
  updateMemberRole,
  useAuth,
  type OrgMember,
  type PendingInvite,
} from "@pylonsync/client";
import { Mail } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Avatar } from "@/components/task-ui";
import { personName } from "@/lib/names";
import {
  FREE_PROJECT_LIMIT,
  TRIAL_DAYS,
  annualSavingsPercent,
  formatPrice,
  planById,
} from "@/lib/plans";
import { inputCls } from "./projects-client";

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
    <section className="rounded-xl border border-zinc-200/80 bg-white">
      <header className="flex items-center justify-between gap-3 border-b border-zinc-100 px-5 py-3">
        <h2 className="text-[13.5px] font-semibold text-zinc-900">{title}</h2>
        {action}
      </header>
      <div className="px-5 py-4">{children}</div>
    </section>
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
    const roster = await listOrgMembers(orgId);
    setMembers(roster);
    // Pending invites are admin-gated server-side; only fetch when we'd be
    // allowed to see them (avoids a guaranteed 403 for plain members). The
    // invites endpoint also returns invites that were already accepted, so
    // drop any whose email already belongs to a member.
    if (canManage) {
      const joined = new Set(roster.map((m) => (m.email ?? "").toLowerCase()));
      const list = await listInvites(orgId);
      setInvites(list.filter((i) => !joined.has(i.email.toLowerCase())));
    }
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

  async function changeRole(userId: string, next: string) {
    setNote(null);
    try {
      await updateMemberRole(orgId, userId, next);
    } catch {
      setNote("Couldn't change that role.");
    } finally {
      void load();
    }
  }

  async function remove(m: OrgMember) {
    const label = m.name || m.email || "this member";
    if (!window.confirm(`Remove ${label} from the workspace?`)) return;
    setNote(null);
    setMembers((prev) => prev?.filter((x) => x.user_id !== m.user_id) ?? null);
    try {
      await removeMember(orgId, m.user_id);
    } catch {
      setNote("Couldn't remove that member.");
    } finally {
      void load();
    }
  }

  return (
    <div className="mx-auto max-w-3xl space-y-6">
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
              const label = personName({ name: m.name, email: m.email }) || "Unknown member";
              const isMe = m.user_id === currentUserId;
              return (
                <li
                  key={m.user_id}
                  className="flex items-center gap-3 py-2.5"
                >
                  <Avatar id={m.user_id} name={label} size="md" />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm font-medium text-zinc-900">
                        {label}
                      </span>
                      {isMe && (
                        <span className="rounded bg-zinc-100 px-1.5 py-0.5 text-[10px] font-medium text-zinc-500">
                          You
                        </span>
                      )}
                    </div>
                    {m.email && label !== m.email && (
                      <div className="truncate text-xs text-zinc-500">
                        {m.email}
                      </div>
                    )}
                  </div>
                  {canManage && !isMe && m.role !== "owner" ? (
                    <>
                      <select
                        value={m.role}
                        onChange={(e) => void changeRole(m.user_id, e.target.value)}
                        aria-label={`Role for ${label}`}
                        className="h-8 rounded-md border border-zinc-300 bg-white px-2 text-[12px] text-zinc-700"
                      >
                        <option value="member">Member</option>
                        <option value="admin">Admin</option>
                      </select>
                      <button
                        type="button"
                        onClick={() => void remove(m)}
                        className="text-[13px] font-medium text-zinc-400 transition-colors hover:text-red-600"
                      >
                        Remove
                      </button>
                    </>
                  ) : (
                    <RoleBadge role={m.role} />
                  )}
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
                <Mail className="size-3.5" strokeWidth={1.75} />
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

export interface AccountInfo {
  id: string;
  email: string;
  displayName?: string | null;
}

export function Settings({
  org,
  role,
  memberCount,
  me,
}: {
  org: OrgInfo | null;
  role: string;
  memberCount: number;
  me: AccountInfo | null;
}) {
  if (!org) return <NoOrg />;
  return (
    <>
      <SettingsView org={org} role={role} memberCount={memberCount} />
      {me ? <AccountSettings me={me} /> : null}
    </>
  );
}

// The signed-in person's own account, below the workspace settings. Display
// name saves through `updateProfile`; the password and delete-account paths
// are the framework's own routes.
function AccountSettings({ me }: { me: AccountInfo }) {
  const [displayName, setDisplayName] = useState(me.displayName ?? "");
  const [savedName, setSavedName] = useState(false);
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [pwNote, setPwNote] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState("");
  const [deleteError, setDeleteError] = useState<string | null>(null);

  async function saveName(e: React.FormEvent) {
    e.preventDefault();
    setBusy("name");
    setSavedName(false);
    try {
      await callFn("updateProfile", { displayName });
      setSavedName(true);
    } finally {
      setBusy(null);
    }
  }

  async function changePassword(e: React.FormEvent) {
    e.preventDefault();
    setBusy("password");
    setPwNote(null);
    try {
      const res = await fetch("/api/auth/password/change", {
        method: "POST",
        headers: { "content-type": "application/json" },
        credentials: "include",
        body: JSON.stringify({ currentPassword: current, newPassword: next }),
      });
      const data = (await res.json().catch(() => ({}))) as { error?: { message?: string } };
      if (!res.ok) throw new Error(data.error?.message ?? "Couldn't change the password.");
      setCurrent("");
      setNext("");
      setPwNote("Password updated.");
    } catch (err) {
      setPwNote(err instanceof Error ? err.message : "Couldn't change the password.");
    } finally {
      setBusy(null);
    }
  }

  async function deleteAccount() {
    if (confirm.trim().toLowerCase() !== me.email.toLowerCase()) return;
    setBusy("delete");
    setDeleteError(null);
    try {
      const res = await fetch("/api/auth/account", { method: "DELETE", credentials: "include" });
      if (!res.ok) {
        const data = (await res.json().catch(() => ({}))) as { error?: { message?: string } };
        throw new Error(data.error?.message ?? "Couldn't delete the account.");
      }
      window.location.assign("/");
    } catch (err) {
      setDeleteError(err instanceof Error ? err.message : "Couldn't delete the account.");
      setBusy(null);
    }
  }

  return (
    <div className="mx-auto mt-6 max-w-2xl space-y-6">
      <Card title="Your account">
        <form onSubmit={saveName} className="space-y-3">
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-zinc-700">Display name</span>
            <div className="flex items-center gap-2">
              <input
                value={displayName}
                onChange={(e) => {
                  setDisplayName(e.target.value);
                  setSavedName(false);
                }}
                placeholder="How teammates see you"
                aria-label="Display name"
                className={inputCls}
              />
              <Button type="submit" size="sm" disabled={busy === "name" || !displayName.trim()}>
                {busy === "name" ? "…" : "Save"}
              </Button>
            </div>
          </label>
          {savedName && <p className="text-xs text-green-600">Name updated.</p>}
          <p className="text-[13px] text-zinc-500">
            Signed in as <span className="font-medium text-zinc-700">{me.email}</span>
          </p>
        </form>
      </Card>

      <Card title="Password">
        <form onSubmit={changePassword} className="space-y-3">
          <input
            type="password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
            placeholder="Current password"
            autoComplete="current-password"
            aria-label="Current password"
            className={inputCls}
          />
          <input
            type="password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            placeholder="New password (10+ characters)"
            autoComplete="new-password"
            minLength={10}
            aria-label="New password"
            className={inputCls}
          />
          {pwNote && <p className="text-xs text-zinc-600">{pwNote}</p>}
          <Button type="submit" size="sm" variant="outline" disabled={busy === "password" || !current || next.length < 10}>
            {busy === "password" ? "…" : "Change password"}
          </Button>
          <p className="text-[12px] text-zinc-400">
            Signed in with an email code or Google? Set a password from the{" "}
            <a href="/forgot-password" className="underline underline-offset-2">reset link</a>.
          </p>
        </form>
      </Card>

      <Card title="Delete account">
        <p className="text-sm text-zinc-600">
          Removes your account and signs you out everywhere. Workspaces you own must be deleted
          or handed to another owner first.
        </p>
        <label className="mt-3 block">
          <span className="mb-1.5 block text-[13px] text-zinc-500">
            Type <span className="font-medium text-zinc-700">{me.email}</span> to confirm
          </span>
          <input
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            aria-label="Confirm email"
            className={inputCls}
          />
        </label>
        {deleteError && <p className="mt-2 text-xs text-red-600">{deleteError}</p>}
        <button
          type="button"
          onClick={() => void deleteAccount()}
          disabled={busy === "delete" || confirm.trim().toLowerCase() !== me.email.toLowerCase()}
          className="mt-3 inline-flex h-9 items-center rounded-lg bg-red-600 px-4 text-[13px] font-medium text-white transition-colors hover:bg-red-700 disabled:opacity-40"
        >
          {busy === "delete" ? "Deleting…" : "Delete my account"}
        </button>
      </Card>
    </div>
  );
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
    <div className="mx-auto max-w-2xl space-y-6">
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
  const [annual, setAnnual] = useState(true);
  const active = subscription && ACTIVE.includes(subscription.status);
  const planLabel = active ? subscription!.plan : "free";
  const pro = planById("pro")!;
  const savings = annualSavingsPercent(pro);

  // `/dashboard/billing?upgrade=pro&interval=annual` (from /pricing or the
  // upgrade dialog) starts checkout without another click.
  useEffect(() => {
    if (active || !canManage) return;
    const params = new URLSearchParams(window.location.search);
    if (params.get("upgrade") !== "pro") return;
    const wantsAnnual = params.get("interval") !== "monthly";
    setAnnual(wantsAnnual);
    window.history.replaceState(null, "", window.location.pathname);
    void startCheckout(wantsAnnual);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
      // errors the same — both mean the Stripe setup isn't finished (set
      // STRIPE_SECRET_KEY + STRIPE_PRICE_PRO). That guidance is for you, the
      // developer; the customer just sees the generic "contact support" line.
      setError(
        /not.?configured|STRIPE_NOT_CONFIGURED|no monthly price|priceId/i.test(
          msg,
        )
          ? "Billing isn't available yet. Contact support and we'll get you set up."
          : msg,
      );
      setBusy(null);
    }
  }

  async function startCheckout(yearly: boolean) {
    await run("upgrade", async () => {
      const res = await callFn<{ url: string }>("createCheckoutSession", {
        plan: "pro",
        referenceId: tenantId,
        annual: yearly,
        successUrl: `${origin()}/dashboard/billing`,
        cancelUrl: `${origin()}/dashboard/billing`,
      });
      window.location.assign(res.url);
    });
  }

  function upgrade() {
    return startCheckout(annual);
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
    <div className="mx-auto max-w-2xl space-y-6">
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
                The free plan: up to {FREE_PROJECT_LIMIT} active projects. Pro is unlimited and
                starts with a {TRIAL_DAYS}-day free trial.
              </p>
            )}
            {active && subscription!.status === "trialing" && subscription!.currentPeriodEnd && (
              <p className="mt-1 text-[13px] text-zinc-500">
                Trial — first charge on {formatDate(subscription!.currentPeriodEnd)} unless cancelled.
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

        {!active && canManage && (
          <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-zinc-100 pt-3 text-[13px]">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setAnnual(false)}
                className={`rounded-md px-2 py-1 ${annual ? "text-zinc-500 hover:text-zinc-900" : "bg-zinc-100 font-medium text-zinc-900"}`}
              >
                Monthly {formatPrice(pro.monthly)}/mo
              </button>
              <button
                type="button"
                onClick={() => setAnnual(true)}
                className={`rounded-md px-2 py-1 ${annual ? "bg-zinc-100 font-medium text-zinc-900" : "text-zinc-500 hover:text-zinc-900"}`}
              >
                Yearly {formatPrice(pro.annualPerMonth ?? pro.monthly)}/mo
                {savings > 0 ? ` · save ${savings}%` : ""}
              </button>
            </div>
            <span className="text-zinc-400">Card required; no charge for {TRIAL_DAYS} days.</span>
          </div>
        )}

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

      {/* Dev setup (not shown to customers): set STRIPE_SECRET_KEY,
          STRIPE_WEBHOOK_SECRET, and STRIPE_PRICE_PRO to go live; point the
          Stripe webhook at /api/fn/stripeWebhook. Billing is wired through
          @pylonsync/stripe — see lib/billing.ts. */}
      <p className="text-xs text-zinc-400">
        Payments and invoices are processed securely by Stripe.
      </p>
    </div>
  );
}
