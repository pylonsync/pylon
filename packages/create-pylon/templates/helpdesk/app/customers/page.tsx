import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { CustomersView } from "./customers-view";

export const metadata: Metadata = {
  title: `Customers · ${BRAND.name}`,
  robots: "noindex",
};

export default function CustomersPage({}: PageProps) {
  return <CustomersView />;
}
