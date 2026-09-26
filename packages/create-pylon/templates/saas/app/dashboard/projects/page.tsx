import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { ProjectsPage, type Project } from "../projects-client";
import type { TaskRow } from "@/lib/tasks";

export const metadata: Metadata = {
  title: "Projects — Acme",
  robots: "noindex",
};

// `/dashboard/projects` — the workspace's projects with task progress,
// resolved on the server and kept live on the client. Auth gate and shell
// chrome come from the dashboard layout.
export default function Page({ auth, response, serverData }: PageProps) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const projectsPromise = serverData.list<Project>("Project");
  const tasksPromise = serverData.list<TaskRow>("Task");
  const projects = use(projectsPromise).filter((p) => p.orgId === auth.tenant_id);
  const tasks = use(tasksPromise).filter((t) => t.orgId === auth.tenant_id);
  return <ProjectsPage tenantId={auth.tenant_id} projects={projects} tasks={tasks} />;
}
