import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProductView } from "./product-view";

export const metadata: Metadata = {
  title: "Product",
  robots: "noindex",
};

/** `app/products/[id]/page.tsx` -> `/products/:id`. */
export default function ProductPage({ params }: PageProps<{ id: string }>) {
  return <ProductView productId={params.id} />;
}
