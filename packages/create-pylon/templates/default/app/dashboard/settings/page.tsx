import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Settings } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Settings — Acme",
  robots: "noindex",
};

// `/dashboard/settings` — workspace settings.
export default function SettingsPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return (
    <DashboardShell active="settings" title="Settings" userEmail={me?.email ?? ""}>
      <Settings tenantId={auth.tenant_id} />
    </DashboardShell>
  );
}
