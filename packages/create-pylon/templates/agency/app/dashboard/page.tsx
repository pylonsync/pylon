import React, { use } from "react";
import { Link, type Metadata, type PageProps } from "@pylonsync/react";
import { siteConfig } from "@/lib/site.config";
import { UserMenu, AgencyDashboard } from "./dashboard-client";

export const metadata: Metadata = {
  title: `Dashboard — ${siteConfig.brand.name}`,
  robots: "noindex",
};

// `app/dashboard/page.tsx` → `/dashboard`. Server-side auth gate only — any
// signed-in user can load the shell. The OWNER gate (a studio is single-tenant:
// only PYLON_OWNER_EMAIL may see the inquiries) lives in the `inquiriesForOwner`
// function, which checks `ctx.env.PYLON_OWNER_EMAIL` — the authoritative server
// env. Non-owners get a clean "owner-only" card from the client when that call
// is denied. (Env-based config is read in the function, not here: an SSR page
// render can't reliably read arbitrary `process.env`.)
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
      <AgencyDashboard userEmail={email} />
    </Shell>
  );
}

// Dashboard chrome: a slim top bar with the logo, a link back to the public
// site, and the account menu (a client island for sign-out).
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
              {brand.name} <span className="text-zinc-400">/ studio</span>
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
