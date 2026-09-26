import React, { use } from "react";
import type { PageAuth, ServerData, SsrResponse } from "@pylonsync/react";
import { DashboardShell, NAV, type ShellProject } from "@/components/dashboard-shell";
import { personName } from "@/lib/names";

// `app/dashboard/layout.tsx` wraps every /dashboard page in the sidebar shell,
// so no page repeats the chrome. The layout gates auth, resolves the chrome's
// data on the server (the person's name, the workspace name, the project
// list), derives the active nav item and title from the request URL, and hands
// the interactive shell (a `"use client"` component) its props.
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
  // No active workspace: render the page bare. /dashboard shows the
  // ProvisionWorkspace flow, and the subpages redirect back to /dashboard.
  if (!auth.tenant_id) {
    return <>{children}</>;
  }
  const path = (url ?? "").split("?")[0];
  // `/dashboard/projects/<id>` shows that project's board; its name becomes
  // the title with Projects as the breadcrumb parent.
  const projectId = path.match(/^\/dashboard\/projects\/([^/]+)$/)?.[1];

  // Every read is started before the first use(): calling them one at a time
  // makes each wait for the last, since use() suspends on the first pending
  // thenable. Never Promise.all them — a new pending promise each render means
  // the page never returns (React error #482).
  const mePromise = serverData.get<{ email?: string; displayName?: string | null }>("User", auth.user_id);
  const orgPromise = serverData.get<{ name?: string }>("Org", auth.tenant_id);
  const projectsPromise = serverData.list<ShellProject>("Project");

  const me = use(mePromise);
  const org = use(orgPromise);
  const projects = use(projectsPromise).filter((p) => p.orgId === auth.tenant_id);

  // Longest matching NAV href wins so /dashboard/projects highlights
  // Projects, not Overview.
  const nav =
    NAV.filter((n) => path === n.href || path.startsWith(n.href + "/")).sort(
      (a, b) => b.href.length - a.href.length,
    )[0] ?? NAV[0];
  const project = projectId ? projects.find((p) => p.id === projectId) : undefined;

  return (
    <DashboardShell
      active={nav.key}
      title={project ? project.name : nav.label}
      parent={project ? { label: "Projects", href: "/dashboard/projects" } : undefined}
      activeProjectId={project?.id}
      tenantId={auth.tenant_id}
      userId={auth.user_id}
      userName={personName({ displayName: me?.displayName, email: me?.email })}
      userEmail={me?.email ?? ""}
      orgName={org?.name}
      projects={projects.map((p) => ({ id: p.id, orgId: p.orgId, name: p.name, status: p.status }))}
    >
      {children}
    </DashboardShell>
  );
}
