import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { UserMenu, SubscriberDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate only — any
// signed-in user can load the shell. The OWNER gate (a newsletter is
// single-tenant: only PYLON_OWNER_EMAIL may see the subscribers) lives in the
// `subscriberStats` function, which checks `ctx.env.PYLON_OWNER_EMAIL` — the
// authoritative server env. Non-owners get a clean "owner-only" card from the
// client when that call is denied. (Env-based config is read in the function,
// not here: an SSR page render can't reliably read arbitrary `process.env`.)
export default function DashboardPage({ auth, response, serverData }: PageProps) {
  // Anonymous visitors and guest sessions (guest_… ids) get bounced to login —
  // the dashboard is for the real, signed-in owner only.
  if (!auth.user_id || auth.user_id.startsWith("guest_")) {
    response.redirect("/login");
    return null;
  }

  // The User self-read policy lets the signed-in user read their own row → the
  // email shown in the account menu (and on the owner-only card).
  const me = use(serverData.get<{ email?: string }>("User", auth.user_id));
  const email = me?.email ?? "";

  return (
    <Shell email={email}>
      <SubscriberDashboard userEmail={email} />
    </Shell>
  );
}

// Dashboard chrome in the site's own look: the cream page, the serif face,
// the headshot and name from the site header, a link back to the public site,
// and the account menu (a client island for sign-out).
function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand, newsletter } = siteConfig;
  return (
    <div className="font-display flex min-h-screen flex-col bg-paper text-ink">
      <header className="border-b border-rule">
        <div className="mx-auto flex h-16 w-full max-w-4xl items-center justify-between px-6">
          <Link href="/" className="flex items-center gap-3">
            <img src="/images/headshot.jpg" alt="" className="size-8 rounded-full object-cover" />
            <span className="text-[17px] font-medium text-ink">{brand.name}</span>
            <span className="hidden text-[15px] italic text-ink-2 sm:inline">{newsletter.name}</span>
          </Link>
          <div className="flex items-center gap-5">
            <Link
              href="/"
              className="inline-flex items-center gap-1 text-[15px] text-ink-2 transition-colors hover:text-ink"
            >
              View site
              <ArrowUpRight aria-hidden className="size-4" />
            </Link>
            <UserMenu email={email} />
          </div>
        </div>
      </header>
      <main className="flex-1">
        <div className="mx-auto w-full max-w-4xl px-6 py-10 sm:py-14">{children}</div>
      </main>
    </div>
  );
}
