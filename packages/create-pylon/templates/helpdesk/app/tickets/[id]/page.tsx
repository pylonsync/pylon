import React from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { TicketView } from "./ticket-view";

export const metadata: Metadata = {
  title: "Ticket",
  robots: "noindex",
};

/** `app/tickets/[id]/page.tsx` -> `/tickets/:id`. */
export default function TicketPage({ params }: PageProps<{ id: string }>) {
  return <TicketView ticketId={params.id} />;
}
