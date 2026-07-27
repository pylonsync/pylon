import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { CustomersView } from "./customers-view";

export const metadata: Metadata = {
  title: "Customers",
  robots: "noindex",
};

export default function CustomersPage({}: PageProps) {
  return <CustomersView />;
}
