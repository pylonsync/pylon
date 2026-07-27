import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ProductsView } from "./products-view";

export const metadata: Metadata = {
  title: "Products",
  robots: "noindex",
};

/**
 * `/` — the stock list.
 *
 * Server-side auth gate only. Reading `auth` opts this render out of caching,
 * which is correct: every screen here is private.
 */
export default function ProductsPage({
  auth,
  response,
  searchParams,
  serverData,
}: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return (
    <ProductsView
      email={me?.email ?? ""}
      openNew={searchParams?.new === "product"}
      initialFilter={searchParams?.filter}
    />
  );
}
