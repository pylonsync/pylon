import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { ProductView } from "./product-view";

export const metadata: Metadata = {
  title: `Product · ${BRAND.name}`,
  robots: "noindex",
};

/** `app/products/[id]/page.tsx` -> `/products/:id`. */
export default function ProductPage({ params }: PageProps<{ id: string }>) {
  return <ProductView productId={params.id} />;
}
