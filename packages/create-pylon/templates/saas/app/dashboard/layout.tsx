import React, { use } from "react";
import type { PageAuth, ServerData, SsrResponse } from "@pylonsync/react";
import { DashboardShell, NAV } from "@/components/dashboard-shell";

// `app/dashboard/layout.tsx` wraps every /dashboard page in the sidebar +
// top-bar shell, so no page repeats the chrome. This is the thin-server-
// wrapper pattern: the layout gates auth, resolves the chrome's data
// server-side (user email, org name), derives the active nav item + title
// from the request URL, and hands the interactive shell (a `"use client"`
// component) its props.
interface LayoutProps {
  children: React.ReactNode;
  url: string;
  auth: PageAuth;
  response: SsrResponse;
  serverData: ServerData;
}

export default function DashboardLayout({
  children,
  url,
  auth,
  response,
  serverData,
}: LayoutProps) {
  // The layout renders before any page below it, so gating here protects the
  // whole /dashboard section: signed-out visitors get a real 307 to /login
  // and no page code runs.
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  // No active workspace: render the page bare — /dashboard shows the
  // full-screen ProvisionWorkspace flow, and the subpages redirect back to
  // /dashboard themselves. The shell's org switcher has nothing to show yet.
  if (!auth.tenant_id) {
    return <>{children}</>;
  }
  // Every read is started before the first use(): calling them one at a time
  // makes each wait for the last, since use() suspends on the first pending
  // thenable. Never Promise.all them — a new pending promise each render means
  // the page never returns (React error #482).
  const mePromise = serverData.get<{ email?: string }>("User", auth.user_id);
  const orgPromise = serverData.get<{ name?: string }>("Org", auth.tenant_id);

  const me = use(mePromise);
  const org = use(orgPromise);
  // Longest matching NAV href wins so /dashboard/projects highlights
  // Projects, not Overview.
  const path = (url ?? "").split("?")[0];
  const nav =
    NAV.filter((n) => path === n.href || path.startsWith(n.href + "/")).sort(
      (a, b) => b.href.length - a.href.length,
    )[0] ?? NAV[0];
  return (
    <DashboardShell
      active={nav.key}
      title={nav.label}
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      {children}
    </DashboardShell>
  );
}
