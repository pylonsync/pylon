import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
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

// Dashboard chrome in the site's own look: the cream page, the ruled header,
// the shop name in the display face, a link back to the site, and the account
// menu (a client island for sign-out).
function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-cream text-ink">
      <header className="sticky top-0 z-30 border-b border-ink bg-cream">
        <div className="mx-auto flex h-14 w-full max-w-5xl items-center justify-between px-6">
          <Link href="/" className="font-display text-[26px] leading-none tracking-[0.02em]">
            {brand.name}
          </Link>
          <div className="flex items-center gap-4">
            <Link
              href="/"
              className="hidden items-center gap-1 text-[13px] font-medium text-ink/70 transition-colors hover:text-ink sm:inline-flex"
            >
              View site
              <ArrowUpRight aria-hidden className="size-3.5" />
            </Link>
            <UserMenu email={email} />
          </div>
        </div>
      </header>
      <main className="flex-1">
        <div className="mx-auto w-full max-w-5xl px-6 py-10 sm:py-12">{children}</div>
      </main>
    </div>
  );
}
