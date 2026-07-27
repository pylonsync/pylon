"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Sidebar } from "@/components/sidebar";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";
import { counts, isOpen } from "@/lib/tickets";

export interface CustomerRow {
  id: string;
  name: string;
  email: string;
  company?: string | null;
  createdAt?: string | null;
}
export interface TicketRow {
  id: string;
  subject: string;
  status: string;
  priority: string;
  customerId?: string | null;
  assigneeId?: string | null;
  firstRespondedAt?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}
export interface MessageRow {
  id: string;
  ticketId: string;
  body: string;
  fromCustomer?: boolean | null;
  internal?: boolean | null;
  authorId?: string | null;
  createdAt?: string | null;
}
export interface UserRow {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface WorkspaceData {
  customers: CustomerRow[];
  tickets: TicketRow[];
  messages: MessageRow[];
  agents: UserRow[];
  customerName: (id: string | null | undefined) => string | null;
  agentName: (id: string | null | undefined) => string | null;
  loading: boolean;
}

/**
 * The app shell, and the ONLY place that touches `db`.
 *
 * Every view below receives plain data through the render prop and reports
 * changes through callbacks, which is what lets the components in `components/`
 * render from fixtures in a test with nothing mocked.
 *
 * Each `db.useQuery` is live: a ticket another agent assigns, or a reply they
 * send, arrives here and re-renders the queue. Your own writes take the same
 * path, so there's one behaviour to reason about rather than two.
 */
export function Workspace({
  pathname,
  children,
}: {
  pathname: string;
  children: (data: WorkspaceData) => React.ReactNode;
}) {
  const router = useRouter();
  // The signed-in identity comes from the CLIENT session, not from an SSR
  // prop: the session cookie is SameSite=Lax and browsers withhold it inside
  // the builder's cross-site preview iframe, so a server-resolved email would
  // be empty exactly where this app is most often viewed.
  const { signOut, userId } = useAuth();
  const { data: tickets, loading } = db.useQuery<TicketRow>("Ticket");
  const { data: customers } = db.useQuery<CustomerRow>("Customer");
  const { data: messages } = db.useQuery<MessageRow>("Message");
  const { data: users } = db.useQuery<UserRow>("User");

  // Whoever is signed in, named from the synced User row rather than an
  // SSR prop — see the note on useAuth above.
  const signedInEmail =
    (users ?? []).find((user) => user.id === userId)?.email ?? "";

  const [paletteOpen, setPaletteOpen] = useState(false);
  const seeded = useRef(false);

  const ticketList = tickets ?? [];
  const customerList = customers ?? [];
  const agentList = users ?? [];

  // A brand-new helpdesk gets a realistic queue once, so the inbox never opens
  // empty. `seedWorkspace` no-ops when anything exists; the ref stops a
  // re-render from firing a second call mid-flight.
  useEffect(() => {
    if (loading || seeded.current) return;
    if (ticketList.length > 0) {
      seeded.current = true;
      return;
    }
    seeded.current = true;
    callFn("seedWorkspace", {}).catch(() => {
      // A failed seed leaves an empty inbox, which is usable — the queue has an
      // empty state that offers to open the first ticket.
      seeded.current = false;
    });
  }, [loading, ticketList.length]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing =
        !!target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable);
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
      } else if (event.key === "/" && !typing) {
        event.preventDefault();
        setPaletteOpen(true);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const lookups = useMemo(() => {
    const customerById = new Map(customerList.map((c) => [c.id, c] as const));
    const userById = new Map(agentList.map((u) => [u.id, u] as const));
    return { customerById, userById };
  }, [customerList, agentList]);

  const searchIndex = useMemo<SearchItem[]>(
    () => [
      ...ticketList.map((ticket) => ({
        id: `ticket:${ticket.id}`,
        type: "deal" as const,
        title: ticket.subject,
        subtitle: lookups.customerById.get(ticket.customerId ?? "")?.name,
        href: `/tickets/${ticket.id}`,
      })),
      ...customerList.map((customer) => ({
        id: `customer:${customer.id}`,
        type: "contact" as const,
        title: customer.name,
        subtitle: customer.company ?? undefined,
        href: "/customers",
        keywords: customer.email,
      })),
    ],
    [ticketList, customerList, lookups],
  );

  const queue = counts(ticketList);

  const data: WorkspaceData = {
    customers: customerList,
    tickets: ticketList,
    messages: messages ?? [],
    agents: agentList,
    customerName: (id) => lookups.customerById.get(id ?? "")?.name ?? null,
    agentName: (id) => {
      const user = lookups.userById.get(id ?? "");
      return user?.displayName || user?.email || null;
    },
    loading,
  };

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar
        workspace="Helpdesk"
        email={signedInEmail}
        pathname={pathname}
        counts={{
          "/": ticketList.filter(isOpen).length,
          "/customers": customerList.length,
        }}
        onOpenCommand={() => setPaletteOpen(true)}
        onSignOut={() => void signOut()}
      />
      <main className="flex min-w-0 flex-1 flex-col">{children(data)}</main>

      <CommandPalette
        open={paletteOpen}
        items={searchIndex}
        actions={[
          {
            id: "new-ticket",
            label: "New ticket",
            run: () => router.push("/?new=ticket"),
          },
          {
            id: "unassigned",
            label: `Unassigned (${queue.unassigned})`,
            run: () => router.push("/?filter=unassigned"),
          },
          {
            id: "go-customers",
            label: "Go to Customers",
            run: () => router.push("/customers"),
          },
        ]}
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </div>
  );
}
