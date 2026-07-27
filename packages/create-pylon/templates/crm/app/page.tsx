import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { PipelineView } from "./pipeline-view";

export const metadata: Metadata = {
  title: "Pipeline",
  robots: "noindex",
};

/**
 * `/` — the pipeline board.
 *
 * Server-side auth gate only. Reading `auth` opts this render out of caching,
 * which is correct: every screen in this app is private.
 *
 * The signed-in email comes from the User row, not from `auth` — `PageAuth` is
 * `{ user_id, is_admin, tenant_id, roles }` and carries no email.
 */
export default function PipelinePage({
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
    <PipelineView
      email={me?.email ?? ""}
      // The ⌘K "New deal" action navigates here with ?new=deal.
      openNew={searchParams?.new === "deal"}
    />
  );
}
