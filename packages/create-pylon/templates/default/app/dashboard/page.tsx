import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Overview } from "./dashboard-client";

export const metadata: Metadata = {
  title: "Dashboard — Acme",
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate: anonymous
// requests get a 307 to /login before any HTML is sent (works with JS off, no
// flash). The shell (sidebar + top bar) is the dashboard's own chrome — the
// marketing nav/footer are suppressed for /dashboard in the root layout.
export default function DashboardPage({ auth, response }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <DashboardShell active="overview" title="Overview">
      <Overview />
    </DashboardShell>
  );
}
