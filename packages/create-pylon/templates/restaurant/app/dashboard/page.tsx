import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { UserMenu, ReservationsDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate; the OWNER gate
// (only PYLON_OWNER_EMAIL sees reservations + guest PII) lives in the
// `reservationsForOwner` function via `ctx.env`. Reading `auth` opts the render
// out of caching — correct, the dashboard is private + noindex.
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
      <ReservationsDashboard userEmail={email} />
    </Shell>
  );
}

function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-[var(--ink)] text-[var(--cream)]">
      <header className="sticky top-0 z-30 border-b border-white/10 bg-[var(--ink)]/90 backdrop-blur">
        <div className="mx-auto flex h-16 w-full max-w-5xl items-center justify-between px-6">
          <Link href="/" className="font-display text-[22px] tracking-tight">
            {brand.name}
          </Link>
          <div className="flex items-center gap-4">
            <Link
              href="/"
              className="hidden items-center gap-1 text-[13px] font-medium text-[var(--cream-2)] transition-colors hover:text-[var(--cream)] sm:inline-flex"
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
