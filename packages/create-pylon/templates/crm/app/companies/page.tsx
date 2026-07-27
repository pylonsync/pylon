import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { CompaniesView } from "./companies-view";

export const metadata: Metadata = {
  title: "Companies",
  robots: "noindex",
};

export default function CompaniesPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <CompaniesView email={me?.email ?? ""} />;
}
