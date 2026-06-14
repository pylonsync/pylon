import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Workspace } from "./dashboard-client";

export const metadata: Metadata = {
  title: "Dashboard — Acme",
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate: anonymous
// requests get a 307 to /login before any HTML is sent (works with JS off, no
// flash of the dashboard). The redirect fires — and we return — in the
// synchronous shell render. The workspace itself is a client island that reads
// your active org from the session.
export default function DashboardPage({ auth, response }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <div className="mx-auto w-full max-w-5xl space-y-6 px-6 py-12">
      <h1 className="text-2xl font-semibold tracking-tight">Workspace</h1>
      <Workspace />
    </div>
  );
}
