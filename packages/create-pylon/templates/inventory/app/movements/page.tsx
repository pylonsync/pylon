import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { MovementsView } from "./movements-view";

export const metadata: Metadata = {
  title: "Movements",
  robots: "noindex",
};

export default function MovementsPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <MovementsView email={me?.email ?? ""} />;
}
