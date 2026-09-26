import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { InboxView } from "./inbox-view";

export const metadata: Metadata = {
  title: `Inbox · ${BRAND.name}`,
  robots: "noindex",
};

/** `/` — the support queue. */
export default function InboxPage({ searchParams }: PageProps) {
  return (
    <InboxView
      openNew={searchParams?.new === "ticket"}
      initialFilter={searchParams?.filter}
    />
  );
}
