import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { UserMenu, BookingsDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate only; the OWNER
// gate (only PYLON_OWNER_EMAIL sees the bookings + customer PII) lives in the
// `bookingsForOwner` function via `ctx.env`. A non-owner gets a clean
// "owner-only" card from the client. Reading `auth` here opts the render out of
// caching, which is correct — the dashboard is private + noindex.
export default function DashboardPage({ auth, response, serverData }: PageProps) {
  // Anonymous visitors and guest sessions (guest_… ids) get bounced to login —
  // the dashboard is for the real, signed-in owner only.
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const email = me?.email ?? "";

  return (
    <Shell email={email}>
      <BookingsDashboard userEmail={email} />
    </Shell>
  );
}

function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-white text-zinc-900">
      <header className="border-b border-zinc-200">
        <div className="mx-auto flex h-14 max-w-4xl items-center justify-between px-6">
          <div className="flex items-center gap-2">
            <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
              {brand.letter}
            </span>
            <span className="text-[15px] font-semibold tracking-tight">
              {brand.name} <span className="text-zinc-400">/ bookings</span>
            </span>
          </div>
          <div className="flex items-center gap-4">
            <Link
              href="/"
              className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
            >
              View site ↗
            </Link>
            <UserMenu email={email} />
          </div>
        </div>
      </header>
      <main className="flex-1">
        <div className="mx-auto max-w-4xl px-6 py-8">{children}</div>
      </main>
    </div>
  );
}
