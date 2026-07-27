import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { InboxView } from "./inbox-view";

export const metadata: Metadata = {
  title: "Inbox",
  robots: "noindex",
};

/**
 * `/` — the support queue.
 *
 * Server-side auth gate only. Reading `auth` opts this render out of caching,
 * which is correct: every screen here is private.
 *
 * The signed-in email comes from the User row, not from `auth` — `PageAuth` is
 * `{ user_id, is_admin, tenant_id, roles }` and carries no email.
 */
export default function InboxPage({
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
    <InboxView
      email={me?.email ?? ""}
      openNew={searchParams?.new === "ticket"}
      initialFilter={searchParams?.filter}
    />
  );
}
