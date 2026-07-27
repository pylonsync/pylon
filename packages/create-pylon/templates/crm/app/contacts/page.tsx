import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ContactsView } from "./contacts-view";

export const metadata: Metadata = {
  title: "Contacts",
  robots: "noindex",
};

export default function ContactsPage({ auth, response, serverData }: PageProps) {
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  return <ContactsView email={me?.email ?? ""} />;
}
