import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Projects, type Project } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Projects — Acme",
  robots: "noindex",
};

// `/dashboard/projects` — this org's projects, server-resolved via `use()` then
// kept live + optimistic by `db` on the client. Auth gate + shell chrome come
// from the dashboard layout.
export default function ProjectsPage({ auth, response, serverData }: PageProps) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const projects = use(serverData.list<Project>("Project"));
  return <Projects tenantId={auth.tenant_id} initial={projects} />;
}
