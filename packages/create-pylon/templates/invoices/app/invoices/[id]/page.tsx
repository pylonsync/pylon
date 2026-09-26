import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { InvoiceView } from "./invoice-view";

export const metadata: Metadata = {
  title: `Invoice · ${BRAND.name}`,
  robots: "noindex",
};

/** `app/invoices/[id]/page.tsx` -> `/invoices/:id`. */
export default function InvoicePage({ params }: PageProps<{ id: string }>) {
  return <InvoiceView invoiceId={params.id} />;
}
