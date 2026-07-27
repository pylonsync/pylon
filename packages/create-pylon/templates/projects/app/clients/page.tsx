import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ClientsView } from "./clients-view";

export const metadata: Metadata = {
  title: "Clients",
  robots: "noindex",
};

export default function ClientsPage({}: PageProps) {
  return <ClientsView />;
}
