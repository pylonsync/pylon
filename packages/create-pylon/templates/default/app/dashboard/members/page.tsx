import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Members, type OrgMemberRow } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Members — Acme",
  robots: "noindex",
};

// `/dashboard/members` — org members (server-resolved via `use()`) + invites,
// which the framework gates to org admins.
export default function MembersPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const members = use(serverData.list<OrgMemberRow>("OrgMember"));
  return (
    <DashboardShell active="members" title="Members" userEmail={me?.email ?? ""}>
      <Members tenantId={auth.tenant_id} initial={members} />
    </DashboardShell>
  );
}
