import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProjectView } from "./project-view";

export const metadata: Metadata = {
  title: "Project",
  robots: "noindex",
};

/** `app/projects/[id]/page.tsx` -> `/projects/:id`. */
export default function ProjectPage({
  auth,
  response,
  params,
  serverData,
}: PageProps<{ id: string }>) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <ProjectView email={me?.email ?? ""} projectId={params.id} />;
}
