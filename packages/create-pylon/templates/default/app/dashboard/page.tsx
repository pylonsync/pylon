import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Overview, type Project, type OrgMemberRow } from "./dashboard-client";

export const metadata: Metadata = {
  title: "Dashboard — Acme",
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate, then the
// active org (`auth.tenant_id`) + this org's rows are read during the render
// via `serverData` + React 19 `use()` — resolved server-side and replayed on
// hydration, so the dashboard paints with real data on the first byte (no
// client fetch, no empty-state flash). The marketing nav/footer are suppressed
// for /dashboard in the root layout; the shell is the only chrome here.
export default function DashboardPage({
  auth,
  response,
  serverData,
}: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const org = auth.tenant_id
    ? use(serverData.get<{ name?: string }>("Org", auth.tenant_id))
    : null;
  const projects = use(serverData.list<Project>("Project"));
  const members = use(serverData.list<OrgMemberRow>("OrgMember"));
  // The OrgMember read policy returns this user's memberships across every org,
  // so scope the count to the active workspace.
  const memberCount = members.filter((m) => m.orgId === auth.tenant_id).length;
  return (
    <DashboardShell
      active="overview"
      title="Overview"
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      <Overview
        tenantId={auth.tenant_id}
        projects={projects}
        memberCount={memberCount}
      />
    </DashboardShell>
  );
}
