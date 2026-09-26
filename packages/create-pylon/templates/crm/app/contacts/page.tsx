import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { BRAND } from "@/lib/brand";
import { ContactsView } from "./contacts-view";

export const metadata: Metadata = {
  title: `Contacts · ${BRAND.name}`,
  robots: "noindex",
};

export default function ContactsPage({}: PageProps) {
  return <ContactsView />;
}
