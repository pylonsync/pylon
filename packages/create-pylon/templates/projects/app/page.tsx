import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProjectsView } from "./projects-view";

export const metadata: Metadata = {
  title: "Projects",
  robots: "noindex",
};

/**
 * `/` — the project list.
 *
 * Server-side auth gate only. Reading `auth` opts this render out of caching,
 * which is correct: every screen here is private.
 */
export default function ProjectsPage({
  auth,
  response,
  searchParams,
  serverData,
}: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return (
    <ProjectsView email={me?.email ?? ""} openNew={searchParams?.new === "project"} />
  );
}
