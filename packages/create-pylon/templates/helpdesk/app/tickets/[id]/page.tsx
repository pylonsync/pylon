import React, { use } from "react";
import type { Metadata, PageProps } from "@pylonsync/react";
import { TicketView } from "./ticket-view";

export const metadata: Metadata = {
  title: "Ticket",
  robots: "noindex",
};

/** `app/tickets/[id]/page.tsx` -> `/tickets/:id`. */
export default function TicketPage({
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
  return <TicketView email={me?.email ?? ""} ticketId={params.id} />;
}
