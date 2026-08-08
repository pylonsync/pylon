import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Members } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Members — Acme",
  robots: "noindex",
};

// `/dashboard/members` — the active org's roster + invites. The roster (with
// real emails) is loaded client-side from the framework's org-members endpoint;
// invites are gated to owners/admins both here and on the server. Auth gate +
// shell chrome come from the dashboard layout.
export default function MembersPage({ auth, response }: PageProps) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  return (
    <Members
      tenantId={auth.tenant_id}
      currentUserId={auth.user_id!}
      role={auth.roles?.[0] ?? ""}
    />
  );
}
