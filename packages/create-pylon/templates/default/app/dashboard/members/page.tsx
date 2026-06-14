import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Members } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Members — Acme",
  robots: "noindex",
};

// `/dashboard/members` — the active org's roster + invites. The roster (with
// real emails) is loaded client-side from the framework's org-members endpoint;
// invites are gated to owners/admins both here and on the server.
export default function MembersPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const org = auth.tenant_id
    ? use(serverData.get<{ name?: string }>("Org", auth.tenant_id))
    : null;
  return (
    <DashboardShell
      active="members"
      title="Members"
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      <Members
        tenantId={auth.tenant_id}
        currentUserId={auth.user_id}
        role={auth.roles?.[0] ?? ""}
      />
    </DashboardShell>
  );
}
