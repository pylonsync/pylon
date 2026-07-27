import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProjectsView } from "./projects-view";

export const metadata: Metadata = {
  title: "Projects",
  robots: "noindex",
};

/**
 * `/` — the project list.
 *
 */
export default function ProjectsPage({ searchParams }: PageProps) {
  return (
    <ProjectsView openNew={searchParams?.new === "project"} />
  );
}
