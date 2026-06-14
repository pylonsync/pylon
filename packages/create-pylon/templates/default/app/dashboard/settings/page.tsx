import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import {
  Settings,
  type OrgInfo,
  type OrgMemberRow,
} from "../dashboard-client";

export const metadata: Metadata = {
  title: "Settings — Acme",
  robots: "noindex",
};

// `/dashboard/settings` — workspace settings: rename (owners/admins) and delete
// (owners). The active org + member count are resolved server-side so the page
// paints with real values on the first byte.
export default function SettingsPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const org = auth.tenant_id
    ? use(serverData.get<OrgInfo>("Org", auth.tenant_id))
    : null;
  const members = use(serverData.list<OrgMemberRow>("OrgMember"));
  const memberCount = members.filter(
    (m) => m.orgId === auth.tenant_id,
  ).length;
  return (
    <DashboardShell
      active="settings"
      title="Settings"
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      <Settings
        org={org}
        role={auth.roles?.[0] ?? ""}
        memberCount={memberCount}
      />
    </DashboardShell>
  );
}
