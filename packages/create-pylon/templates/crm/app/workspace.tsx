"use client";

import React, { useEffect, useMemo, useRef, useState } from "react";
import { callFn, db, useRouter } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Sidebar } from "@/components/sidebar";
import { CommandPalette } from "@/components/command-palette";
import type { SearchItem } from "@/lib/search";

export interface CompanyRow {
  id: string;
  name: string;
  domain?: string | null;
  industry?: string | null;
  size?: string | null;
  createdAt?: string | null;
}
export interface ContactRow {
  id: string;
  name: string;
  email?: string | null;
  phone?: string | null;
  title?: string | null;
  companyId?: string | null;
  createdAt?: string | null;
}
export interface DealRow {
  id: string;
  title: string;
  value?: number | null;
  stage: string;
  companyId?: string | null;
  contactId?: string | null;
  closeDate?: string | null;
  ownerId?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}
export interface ActivityRow {
  id: string;
  kind: string;
  body: string;
  dealId?: string | null;
  contactId?: string | null;
  ownerId?: string | null;
  createdAt?: string | null;
}
export interface UserRow {
  id: string;
  email: string;
  displayName?: string | null;
}

export interface WorkspaceData {
  companies: CompanyRow[];
  contacts: ContactRow[];
  deals: DealRow[];
  activities: ActivityRow[];
  companyName: (id: string | null | undefined) => string | null;
  ownerName: (id: string | null | undefined) => string | null;
  loading: boolean;
}

/**
 * The app shell, and the ONLY place that touches `db`.
 *
 * Every screen below receives plain data through the render prop and reports
 * changes through callbacks, which is what keeps the components in
 * `components/` renderable from fixtures in a test with nothing mocked.
 *
 * Each `db.useQuery` is live: when a teammate moves a deal, the row arrives
 * here and the board re-renders. Your own writes take exactly the same path,
 * so there's one behaviour to reason about rather than two.
 */
export function Workspace({
  email,
  pathname,
  children,
}: {
  email: string;
  pathname: string;
  children: (data: WorkspaceData) => React.ReactNode;
}) {
  const router = useRouter();
  const { signOut } = useAuth();
  const { data: companies, loading: loadingCompanies } =
    db.useQuery<CompanyRow>("Company");
  const { data: contacts } = db.useQuery<ContactRow>("Contact");
  const { data: deals } = db.useQuery<DealRow>("Deal");
  const { data: activities } = db.useQuery<ActivityRow>("Activity");
  const { data: users } = db.useQuery<UserRow>("User");

  const [paletteOpen, setPaletteOpen] = useState(false);
  const seeded = useRef(false);

  const companyList = companies ?? [];
  const contactList = contacts ?? [];
  const dealList = deals ?? [];

  // A brand-new workspace gets a realistic pipeline once, so the app never
  // opens on an empty board. `seedWorkspace` no-ops when anything exists, and
  // the ref stops a re-render from firing a second call mid-flight.
  useEffect(() => {
    if (loadingCompanies || seeded.current) return;
    if (companyList.length > 0) {
      seeded.current = true;
      return;
    }
    seeded.current = true;
    callFn("seedWorkspace", {}).catch(() => {
      // A failed seed leaves an empty workspace, which is usable — every view
      // has an empty state that offers to create the first record.
      seeded.current = false;
    });
  }, [loadingCompanies, companyList.length]);

  // ⌘K / Ctrl+K anywhere, and "/" when you're not already typing.
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

  const byId = useMemo(() => {
    const companyById = new Map(companyList.map((c) => [c.id, c] as const));
    const userById = new Map((users ?? []).map((u) => [u.id, u] as const));
    return { companyById, userById };
  }, [companyList, users]);

  const searchIndex = useMemo<SearchItem[]>(
    () => [
      ...dealList.map((deal) => ({
        id: `deal:${deal.id}`,
        type: "deal" as const,
        title: deal.title,
        subtitle: byId.companyById.get(deal.companyId ?? "")?.name,
        href: `/deals/${deal.id}`,
      })),
      ...companyList.map((company) => ({
        id: `company:${company.id}`,
        type: "company" as const,
        title: company.name,
        subtitle: company.domain ?? undefined,
        href: "/companies",
        keywords: company.industry ?? undefined,
      })),
      ...contactList.map((contact) => ({
        id: `contact:${contact.id}`,
        type: "contact" as const,
        title: contact.name,
        subtitle: byId.companyById.get(contact.companyId ?? "")?.name,
        href: "/contacts",
        keywords: `${contact.email ?? ""} ${contact.title ?? ""}`,
      })),
    ],
    [dealList, companyList, contactList, byId],
  );

  const data: WorkspaceData = {
    companies: companyList,
    contacts: contactList,
    deals: dealList,
    activities: activities ?? [],
    companyName: (id) => byId.companyById.get(id ?? "")?.name ?? null,
    ownerName: (id) => {
      const user = byId.userById.get(id ?? "");
      return user?.displayName || user?.email || null;
    },
    loading: loadingCompanies,
  };

  return (
    <div className="flex h-screen overflow-hidden">
      <Sidebar
        workspace="CRM"
        email={email}
        pathname={pathname}
        counts={{
          "/companies": companyList.length,
          "/contacts": contactList.length,
        }}
        onOpenCommand={() => setPaletteOpen(true)}
        onSignOut={async () => {
          await signOut();
          // Full navigation so the SSR runtime re-resolves auth from the
          // cookie and the login page renders server-side.
          window.location.assign("/login");
        }}
      />
      <main className="flex min-w-0 flex-1 flex-col">{children(data)}</main>

      <CommandPalette
        open={paletteOpen}
        items={searchIndex}
        actions={[
          {
            id: "new-deal",
            label: "New deal",
            run: () => router.push("/?new=deal"),
          },
          {
            id: "go-companies",
            label: "Go to Companies",
            run: () => router.push("/companies"),
          },
          {
            id: "go-contacts",
            label: "Go to Contacts",
            run: () => router.push("/contacts"),
          },
        ]}
        onClose={() => setPaletteOpen(false)}
        onSelect={(item) => router.push(item.href)}
      />
    </div>
  );
}
