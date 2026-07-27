import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { CustomersView } from "./customers-view";

export const metadata: Metadata = {
  title: "Customers",
  robots: "noindex",
};

export default function CustomersPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <CustomersView email={me?.email ?? ""} />;
}
