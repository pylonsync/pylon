import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Projects, type Project } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Projects — Acme",
  robots: "noindex",
};

// `/dashboard/projects` — this org's projects, server-resolved via `use()` then
// kept live + optimistic by `db` on the client.
export default function ProjectsPage({
  auth,
  response,
  serverData,
}: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const org = use(serverData.get<{ name?: string }>("Org", auth.tenant_id));
  const projects = use(serverData.list<Project>("Project"));
  return (
    <DashboardShell
      active="projects"
      title="Projects"
      userEmail={me?.email ?? ""}
      orgName={org?.name}
    >
      <Projects tenantId={auth.tenant_id} initial={projects} />
    </DashboardShell>
  );
}
