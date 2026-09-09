import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import {
  Settings,
  type AccountInfo,
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
  // Every read is started before the first use(): calling them one at a time
  // makes each wait for the last, since use() suspends on the first pending
  // thenable. Never Promise.all them — a new pending promise each render means
  // the page never returns (React error #482).
  const orgPromise = serverData.get<OrgInfo>("Org", auth.tenant_id);
  const mePromise = serverData.get<AccountInfo>("User", auth.user_id!);
  const membersPromise = serverData.list<OrgMemberRow>("OrgMember");

  const org = use(orgPromise);
  const me = use(mePromise);
  const memberCount = use(membersPromise).filter(
    (m) => m.orgId === auth.tenant_id,
  ).length;
  return (
    <Settings
      org={org}
      role={auth.roles?.[0] ?? ""}
      memberCount={memberCount}
      me={me}
    />
  );
}
