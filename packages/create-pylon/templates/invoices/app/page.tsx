import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { InvoicesView } from "./invoices-view";

export const metadata: Metadata = {
  title: "Invoices",
  robots: "noindex",
};

/**
 * `/` — the invoice list.
 *
 */
export default function InvoicesPage({ searchParams }: PageProps) {
  return (
    <InvoicesView
          openNew={searchParams?.new === "invoice"}
    />
  );
}
