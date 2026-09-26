import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { ClientsView } from "./clients-view";

export const metadata: Metadata = {
  title: `Clients · ${BRAND.name}`,
  robots: "noindex",
};

export default function ClientsPage({}: PageProps) {
  return <ClientsView />;
}
