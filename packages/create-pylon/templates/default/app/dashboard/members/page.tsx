import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Members } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Members — Acme",
  robots: "noindex",
};

// `/dashboard/members` — org members + invites (admin-gated by the framework).
export default function MembersPage({ auth, response }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <DashboardShell active="members" title="Members">
      <Members />
    </DashboardShell>
  );
}
