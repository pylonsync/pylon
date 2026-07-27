import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { CompaniesView } from "./companies-view";

export const metadata: Metadata = {
  title: "Companies",
  robots: "noindex",
};

export default function CompaniesPage({}: PageProps) {
  return <CompaniesView />;
}
