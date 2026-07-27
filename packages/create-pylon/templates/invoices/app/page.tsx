import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { InvoicesView } from "./invoices-view";

export const metadata: Metadata = {
  title: "Invoices",
  robots: "noindex",
};

/**
 * `/` — the invoice list.
 *
 * Server-side auth gate only. Reading `auth` opts this render out of caching,
 * which is correct: every screen here is private.
 *
 * The signed-in email comes from the User row — `PageAuth` is
 * `{ user_id, is_admin, tenant_id, roles }` and carries no email.
 */
export default function InvoicesPage({
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
    <InvoicesView
      email={me?.email ?? ""}
      openNew={searchParams?.new === "invoice"}
    />
  );
}
