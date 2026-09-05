import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ProvisionWorkspace } from "./provision-workspace";
import {
  Overview,
  type Project,
  type OrgMemberRow,
  type Subscription,
} from "./dashboard-client";
import type { SetupState } from "@/components/setup-checklist";

export const metadata: Metadata = {
  title: "Dashboard — Acme",
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. The dashboard layout owns the auth
// gate and the shell chrome; this page reads the active org's rows during the
// render via `serverData` + React 19 `use()` — resolved server-side and
// replayed on hydration, so the dashboard paints with real data on the first
// byte (no client fetch, no empty-state flash).
export default function DashboardPage({ auth, response, serverData }: PageProps) {
  // No active workspace (signup's auto-provision failed, or the user left/
  // deleted their last org). Every read below is tenant-scoped, so instead of
  // an empty shell, provision one client-side and reload into a ready dashboard.
  // The layout renders this bare (no shell) when there's no tenant.
  if (!auth.tenant_id) {
    return <ProvisionWorkspace />;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id!));
  const org = use(
    serverData.get<{ name?: string; onboardedAt?: string | null; setupDismissedAt?: string | null }>(
      "Org",
      auth.tenant_id,
    ),
  );
  // A workspace created outside the wizard (or one that abandoned it) gets
  // sent through it once. Owners/admins only; a member who lands here just
  // sees the dashboard.
  const role = auth.roles?.[0] ?? "";
  const manages = role === "owner" || role === "admin";
  if (org && !org.onboardedAt && manages) {
    response.redirect("/onboarding");
    return null;
  }
  const projects = use(serverData.list<Project>("Project"));
  const members = use(serverData.list<OrgMemberRow>("OrgMember"));
  // The OrgMember read policy returns this user's memberships across every org,
  // so scope the count to the active workspace.
  const memberCount = members.filter((m) => m.orgId === auth.tenant_id).length;
  // Active-plan badge from the workspace's Stripe subscription (Free until one
  // exists). Scoped to the active tenant by the plugin's read policy.
  const subs = use(serverData.list<Subscription>("StripeSubscription"));
  const active = subs.find((s) =>
    ["active", "trialing", "past_due"].includes(s.status),
  );
  const plan = active ? active.plan : "free";
  // Getting-started checklist, derived from real rows so it ticks itself off.
  const invites = use(serverData.list<{ orgId: string; acceptedAt?: string | null }>("OrgInvite"));
  const pendingInvites = invites.filter((i) => i.orgId === auth.tenant_id && !i.acceptedAt).length;
  const setup: SetupState | null = org?.setupDismissedAt
    ? null
    : {
        orgId: auth.tenant_id,
        hasProject: projects.length > 0,
        hasTeammate: memberCount > 1 || pendingInvites > 0,
        hasBilling: plan !== "free",
        canDismiss: manages,
      };
  return (
    <Overview
      setup={setup}
      tenantId={auth.tenant_id}
      orgName={org?.name}
      userEmail={me?.email}
      projects={projects}
      memberCount={memberCount}
      plan={plan}
    />
  );
}
