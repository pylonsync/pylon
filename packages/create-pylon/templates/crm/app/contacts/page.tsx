import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { ContactsView } from "./contacts-view";

export const metadata: Metadata = {
  title: "Contacts",
  robots: "noindex",
};

export default function ContactsPage({}: PageProps) {
  return <ContactsView />;
}
