import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ProvisionWorkspace } from "./provision-workspace";
import { Overview } from "./overview-client";
import type { Project } from "./projects-client";
import type { OrgMemberRow, Subscription } from "./dashboard-client";
import type { SetupState } from "@/components/setup-checklist";
import { firstName } from "@/lib/names";
import { isoDay, type TaskRow } from "@/lib/tasks";

export const metadata: Metadata = {
  title: "Overview — Acme",
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. The dashboard layout owns the auth
// gate and the shell chrome; this page reads the active workspace's rows
// during the render via `serverData` + React 19 `use()`, so the Overview
// paints with real data on the first byte. The client then keeps projects and
// tasks live.
export default function DashboardPage({ auth, response, serverData }: PageProps) {
  // No active workspace (the user left or deleted their last org, or quit the
  // wizard before step 1). Every read below is tenant-scoped, so select an
  // existing workspace or go to onboarding instead of rendering an empty shell.
  if (!auth.tenant_id) {
    return <ProvisionWorkspace />;
  }
  // Every read is STARTED here, before the first use(). Each serverData call
  // returns a thenable the handle caches by key, and use() suspends on the
  // first one still pending, so reading them one at a time makes each wait
  // for the last. Issued together they overlap.
  //
  // Do NOT reach for Promise.all. It builds a new, pending, uncached promise
  // on every render, so use() suspends, re-renders, builds another, and the
  // page never returns (React error #482).
  const mePromise = serverData.get<{ email?: string; displayName?: string | null }>("User", auth.user_id!);
  const orgPromise = serverData.get<{
    name?: string;
    onboardedAt?: string | null;
    setupDismissedAt?: string | null;
  }>("Org", auth.tenant_id);
  const projectsPromise = serverData.list<Project>("Project");
  const tasksPromise = serverData.list<TaskRow>("Task");
  const membersPromise = serverData.list<OrgMemberRow>("OrgMember");
  const subsPromise = serverData.list<Subscription>("StripeSubscription");
  const invitesPromise =
    serverData.list<{ orgId: string; acceptedAt?: string | null }>("OrgInvite");

  const me = use(mePromise);
  const org = use(orgPromise);
  // A workspace created outside the wizard (or one that abandoned it) gets
  // sent through it once. Owners/admins only; a member who lands here just
  // sees the dashboard.
  const role = auth.roles?.[0] ?? "";
  const manages = role === "owner" || role === "admin";
  if (org && !org.onboardedAt && manages) {
    response.redirect("/onboarding");
    return null;
  }
  const projects = use(projectsPromise).filter((p) => p.orgId === auth.tenant_id);
  const tasks = use(tasksPromise).filter((t) => t.orgId === auth.tenant_id);
  // The OrgMember read policy returns this user's memberships across every org,
  // so scope the count to the active workspace.
  const memberCount = use(membersPromise).filter((m) => m.orgId === auth.tenant_id).length;
  // Plan from the workspace's Stripe subscription (Free until one exists).
  // Scoped to the active tenant by the plugin's read policy.
  const active = use(subsPromise).find((s) =>
    ["active", "trialing", "past_due"].includes(s.status),
  );
  const plan = active ? active.plan : "free";
  // Getting-started checklist, derived from real rows so it ticks itself off.
  const pendingInvites = use(invitesPromise).filter(
    (i) => i.orgId === auth.tenant_id && !i.acceptedAt,
  ).length;
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
      tenantId={auth.tenant_id}
      userId={auth.user_id!}
      firstName={firstName({ displayName: me?.displayName, email: me?.email })}
      projects={projects}
      tasks={tasks}
      memberCount={memberCount}
      plan={plan}
      today={isoDay(new Date())}
      canSeed={manages}
      setup={setup}
    />
  );
}
