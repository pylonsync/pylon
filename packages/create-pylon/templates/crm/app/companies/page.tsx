import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { CompaniesView } from "./companies-view";

export const metadata: Metadata = {
  title: `Companies · ${BRAND.name}`,
  robots: "noindex",
};

export default function CompaniesPage({}: PageProps) {
  return <CompaniesView />;
}
