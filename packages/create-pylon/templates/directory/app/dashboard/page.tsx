import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { ArrowUpRight } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { WRAP } from "@/components/marketing";
import { UserMenu, DirectoryDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate only — any
// signed-in user can load the shell. The OWNER gate (a directory is single-
// tenant: only PYLON_OWNER_EMAIL may see submissions) lives in the
// `submissionsForOwner` function, which checks `ctx.env.PYLON_OWNER_EMAIL` — the
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
      <DirectoryDashboard userEmail={email} />
    </Shell>
  );
}

// Dashboard chrome in the site's own look: the same slim white nav with the
// directory name, a link back to the site, and the account menu (a client
// island for sign-out).
function Shell({ email, children }: { email: string; children: React.ReactNode }) {
  const { brand } = siteConfig;
  return (
    <div className="flex min-h-screen flex-col bg-white text-zinc-900">
      <header className="border-b border-zinc-200">
        <div className={`${WRAP} flex h-12 items-center justify-between`}>
          <div className="flex items-baseline gap-2.5">
            <Link href="/" className="text-[15px] font-semibold text-zinc-900">
              {brand.name}
            </Link>
            <span className="font-mono text-[12px] text-zinc-400">curator</span>
          </div>
          <div className="flex items-center gap-5">
            <Link
              href="/"
              className="hidden items-center gap-1 text-[14px] text-zinc-600 hover:text-zinc-900 sm:inline-flex"
            >
              View site
              <ArrowUpRight aria-hidden className="size-3.5" />
            </Link>
            <UserMenu email={email} />
          </div>
        </div>
      </header>
      <main className="flex-1">
        <div className={`${WRAP} py-10`}>{children}</div>
      </main>
    </div>
  );
}
