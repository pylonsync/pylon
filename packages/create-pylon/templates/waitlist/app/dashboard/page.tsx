import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { UserMenu, WaitlistDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate only — any
// signed-in user can load the shell. The OWNER gate (a waitlist is
// single-tenant: only PYLON_OWNER_EMAIL may see the signups) lives in the
// `waitlistStats` function, which checks `ctx.env.PYLON_OWNER_EMAIL` — the
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
      <WaitlistDashboard userEmail={email} />
    </Shell>
  );
}

// Dashboard chrome in the site's own look: the same near-black page, the brand
// name in the display face, a link back to the public site, and the account
// menu (a client island for sign-out).
function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-ink text-chalk">
      <header className="border-b border-line">
        <div className="mx-auto flex h-16 w-full max-w-5xl items-center justify-between px-6">
          <div className="flex items-baseline gap-2.5">
            <Link href="/" className="font-display text-[17px] font-bold tracking-tight text-chalk">
              {brand.name}
            </Link>
            <span className="font-mono-ui text-[12px] text-chalk-2">owner</span>
          </div>
          <div className="flex items-center gap-5">
            <Link
              href="/"
              className="inline-flex items-center gap-1 text-[13px] text-chalk-2 transition-colors hover:text-chalk"
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
