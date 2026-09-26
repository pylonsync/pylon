import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { ProjectView } from "./project-view";

export const metadata: Metadata = {
  title: `Project · ${BRAND.name}`,
  robots: "noindex",
};

/** `app/projects/[id]/page.tsx` -> `/projects/:id`. */
export default function ProjectPage({ params }: PageProps<{ id: string }>) {
  return <ProjectView projectId={params.id} />;
}
