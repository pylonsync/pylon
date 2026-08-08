import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
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
// paints with real values on the first byte. Auth gate + shell chrome come from
// the dashboard layout.
export default function SettingsPage({ auth, response, serverData }: PageProps) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const org = use(serverData.get<OrgInfo>("Org", auth.tenant_id));
  const members = use(serverData.list<OrgMemberRow>("OrgMember"));
  const memberCount = members.filter(
    (m) => m.orgId === auth.tenant_id,
  ).length;
  return (
    <Settings
      org={org}
      role={auth.roles?.[0] ?? ""}
      memberCount={memberCount}
    />
  );
}
