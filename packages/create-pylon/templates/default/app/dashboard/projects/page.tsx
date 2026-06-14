import React from "react";
import { type Metadata, type PageProps } from "@pylonsync/react";
import { DashboardShell } from "@/components/dashboard-shell";
import { Projects } from "../dashboard-client";

export const metadata: Metadata = {
  title: "Projects — Acme",
  robots: "noindex",
};

// `/dashboard/projects` — tenant-scoped projects for the active org.
export default function ProjectsPage({ auth, response }: PageProps) {
  if (!auth.user_id) {
    response.redirect("/login");
    return null;
  }
  return (
    <DashboardShell active="projects" title="Projects">
      <Projects />
    </DashboardShell>
  );
}
