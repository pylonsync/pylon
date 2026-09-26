"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { FileText, User } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";
import type { Invoice, LineItem, Payment } from "@/lib/billing";

export interface ClientRow {
  id: string;
  name: string;
  email?: string | null;
  address?: string | null;
  taxId?: string | null;
  createdAt?: string | null;
}
export type InvoiceRow = Invoice & { ownerId?: string | null };
export type LineItemRow = LineItem & { position?: number };
export type PaymentRow = Payment & { reference?: string | null };
export interface UserRow {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface WorkspaceData {
  clients: ClientRow[];
  invoices: InvoiceRow[];
  items: LineItemRow[];
  payments: PaymentRow[];
  clientName: (id: string | null | undefined) => string | null;
  /**
   * The number beside each nav link, keyed by href. Page headers read the
   * same map, so the sidebar and the page can't disagree.
   */
  navCounts: Record<string, number>;
  loading: boolean;
}

/**
 * The app shell, and the ONLY place that touches `db`.
 *
 * Every view below receives plain data through the render prop and reports
 * changes through callbacks, which is what lets the components in `components/`
 * render from fixtures in a test with nothing mocked.
 *
 * The queries are live: recording a payment updates the balance and flips the
 * status on every open tab, so two people chasing the same overdue invoice stop
 * the moment one of them marks it paid.
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
  const { data: invoices, loading } = db.useQuery<InvoiceRow>("Invoice");
  const { data: clients } = db.useQuery<ClientRow>("Client");
  const { data: items } = db.useQuery<LineItemRow>("LineItem");
  const { data: payments } = db.useQuery<PaymentRow>("Payment");
  const { data: users } = db.useQuery<UserRow>("User");

  // Whoever is signed in, named from the synced User row rather than an
  // SSR prop — see the note on useAuth above.
  const me = (users ?? []).find((user) => user.id === userId);

  const [paletteOpen, setPaletteOpen] = useState(false);
  const seeded = useRef(false);

  const invoiceList = invoices ?? [];
  const clientList = clients ?? [];

  // A brand-new workspace gets a realistic book once, so the list never opens
  // empty. `seedWorkspace` no-ops when anything exists; the ref stops a
  // re-render from firing a second call mid-flight.
  useEffect(() => {
    if (loading || seeded.current) return;
    if (invoiceList.length > 0) {
      seeded.current = true;
      return;
    }
    seeded.current = true;
    callFn("seedWorkspace", {}).catch(() => {
      seeded.current = false;
    });
  }, [loading, invoiceList.length]);

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

  const clientById = useMemo(
    () => new Map(clientList.map((c) => [c.id, c] as const)),
    [clientList],
  );

  const searchIndex = useMemo<SearchItem[]>(
    () => [
      ...invoiceList.map((invoice) => ({
        id: `invoice:${invoice.id}`,
        type: "invoice",
        title: invoice.number,
        subtitle: clientById.get(invoice.clientId ?? "")?.name,
        href: `/invoices/${invoice.id}`,
      })),
      ...clientList.map((client) => ({
        id: `client:${client.id}`,
        type: "client",
        title: client.name,
        subtitle: client.email ?? undefined,
        href: "/clients",
      })),
    ],
    [invoiceList, clientList, clientById],
  );

  const navCounts = { "/": invoiceList.length, "/clients": clientList.length };

  const data: WorkspaceData = {
    clients: clientList,
    invoices: invoiceList,
    items: items ?? [],
    payments: payments ?? [],
    clientName: (id) => clientById.get(id ?? "")?.name ?? null,
    navCounts,
    loading,
  };

  return (
    <AppShell
      sidebar={{
        user: { name: me?.displayName ?? "", email: me?.email ?? "" },
        pathname,
        counts: navCounts,
        onOpenCommand: () => setPaletteOpen(true),
        onSignOut: () => void signOut(),
      }}
    >
      {children(data)}

      <CommandPalette
        open={paletteOpen}
        items={searchIndex}
        actions={[
          {
            id: "new-invoice",
            label: "New invoice",
            run: () => router.push("/?new=invoice"),
          },
          {
            id: "go-clients",
            label: "Go to Clients",
            run: () => router.push("/clients"),
          },
        ]}
        placeholder="Search invoices and clients…"
        icons={{ invoice: <FileText />, client: <User /> }}
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </AppShell>
  );
}
