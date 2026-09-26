import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { ProductsView } from "./products-view";

export const metadata: Metadata = {
  title: `Products · ${BRAND.name}`,
  robots: "noindex",
};

/** `/` — the stock list. */
export default function ProductsPage({ searchParams }: PageProps) {
  return (
    <ProductsView
      openNew={searchParams?.new === "product"}
      initialFilter={searchParams?.filter}
    />
  );
}
