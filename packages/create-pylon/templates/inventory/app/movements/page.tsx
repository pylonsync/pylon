import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { MovementsView } from "./movements-view";

export const metadata: Metadata = {
  title: `Movements · ${BRAND.name}`,
  robots: "noindex",
};

export default function MovementsPage({}: PageProps) {
  return <MovementsView />;
}
