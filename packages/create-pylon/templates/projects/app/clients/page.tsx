import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ClientsView } from "./clients-view";

export const metadata: Metadata = {
  title: "Clients",
  robots: "noindex",
};

export default function ClientsPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <ClientsView email={me?.email ?? ""} />;
}
