import React, { use } from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { Board } from "../../board-client";
import type { Project } from "../../projects-client";
import { isoDay, type TaskRow } from "@/lib/tasks";

export const metadata: Metadata = {
  title: "Project — Acme",
  robots: "noindex",
};

// `/dashboard/projects/:id` — one project's task board. The Project read
// policy only returns rows from the active workspace, so an id from another
// workspace resolves to null and becomes a real 404.
export default function ProjectBoardPage({ auth, params, response, serverData }: PageProps<{ id: string }>) {
  if (!auth.tenant_id) {
    response.redirect("/dashboard");
    return null;
  }
  const projectPromise = serverData.get<Project>("Project", params.id);
  const tasksPromise = serverData.query<TaskRow>("Task", { projectId: params.id });
  const project = use(projectPromise);
  if (!project || project.orgId !== auth.tenant_id) {
    response.notFound();
    return null;
  }
  const tasks = use(tasksPromise).filter((t) => t.orgId === auth.tenant_id);
  return <Board tenantId={auth.tenant_id} project={project} tasks={tasks} today={isoDay(new Date())} />;
}
