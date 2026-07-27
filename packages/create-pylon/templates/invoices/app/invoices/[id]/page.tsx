import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { InvoiceView } from "./invoice-view";

export const metadata: Metadata = {
  title: "Invoice",
  robots: "noindex",
};

/** `app/invoices/[id]/page.tsx` -> `/invoices/:id`. */
export default function InvoicePage({
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
  return <InvoiceView email={me?.email ?? ""} invoiceId={params.id} />;
}
