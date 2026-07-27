import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProductView } from "./product-view";

export const metadata: Metadata = {
  title: "Product",
  robots: "noindex",
};

/** `app/products/[id]/page.tsx` -> `/products/:id`. */
export default function ProductPage({
  auth,
  response,
  params,
  serverData,
}: PageProps<{ id: string }>) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <ProductView email={me?.email ?? ""} productId={params.id} />;
}
