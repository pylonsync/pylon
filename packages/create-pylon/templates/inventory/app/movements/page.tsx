import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { MovementsView } from "./movements-view";

export const metadata: Metadata = {
  title: "Movements",
  robots: "noindex",
};

export default function MovementsPage({}: PageProps) {
  return <MovementsView />;
}
