import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Settings } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Settings — Acme",
  robots: "noindex",
};

// `/dashboard/settings` — workspace settings.
export default function SettingsPage({ auth, response }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <DashboardShell active="settings" title="Settings">
      <Settings />
    </DashboardShell>
  );
}
