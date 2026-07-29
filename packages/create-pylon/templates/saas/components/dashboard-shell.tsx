"use client";

import React from "react";
import { Link, useRouter } from "@pylonsync/react";
import { useAuth, OrganizationSwitcher } from "@pylonsync/client";
import {
  LayoutDashboard,
  FolderKanban,
  Users,
  CreditCard,
  Settings as SettingsIcon,
  LogOut,
  ExternalLink,
  type LucideIcon,
} from "lucide-react";

type NavKey = "overview" | "projects" | "members" | "billing" | "settings";

const NAV: { key: NavKey; label: string; href: string; Icon: LucideIcon }[] = [
  { key: "overview", label: "Overview", href: "/dashboard", Icon: LayoutDashboard },
  { key: "projects", label: "Projects", href: "/dashboard/projects", Icon: FolderKanban },
  { key: "members", label: "Members", href: "/dashboard/members", Icon: Users },
  { key: "billing", label: "Billing", href: "/dashboard/billing", Icon: CreditCard },
  { key: "settings", label: "Settings", href: "/dashboard/settings", Icon: SettingsIcon },
];

// Dashboard chrome: a fixed sidebar (logo, workspace switcher, nav) plus a top
// bar with a user menu. The marketing nav/footer are suppressed for /dashboard
// in the root layout, so this shell is the only chrome here. `userEmail` is
// resolved on the server (serverData.get("User", …)) and passed in, so the menu
// shows a real email instead of a raw id.
export function DashboardShell({
  active,
  title,
  userEmail,
  orgName,
  children,
}: {
  active: NavKey;
  title: string;
  userEmail: string;
  // Active org name, resolved on the server, so the workspace switcher renders
  // the real name on first paint instead of flashing in after hydration.
  orgName?: string;
  children: React.ReactNode;
}) {
  const router = useRouter();
  return (
    <div className="flex min-h-screen bg-white text-zinc-900">
      <aside className="hidden w-60 shrink-0 flex-col border-r border-zinc-200 bg-zinc-50/60 md:flex">
        <div className="flex h-14 items-center border-b border-zinc-200 px-4">
          <Link href="/" className="flex items-center gap-2">
            <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
              A
            </span>
            <span className="text-[15px] font-semibold tracking-tight">Acme</span>
          </Link>
        </div>

        <div className="px-3 py-3">
          {/* Every view's data is resolved server-side for the active tenant,
              so switching orgs re-renders the dashboard for the new workspace.
              A soft client navigation (router.push) re-fetches the SSR page —
              all data updates — without the full-reload white flash. */}
          <OrganizationSwitcher
            hidePersonal
            initialActiveName={orgName}
            onSwitched={() => router.push("/dashboard")}
          />
        </div>

        <nav className="flex-1 space-y-0.5 px-3">
          {NAV.map((n) => {
            const isActive = active === n.key;
            return (
              <Link
                key={n.key}
                href={n.href}
                className={
                  "flex items-center gap-2.5 rounded-md px-3 py-2 text-[13.5px] font-medium transition-colors " +
                  (isActive
                    ? "bg-zinc-900 text-white"
                    : "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900")
                }
              >
                <n.Icon
                  className={
                    "size-[17px] " + (isActive ? "text-white" : "text-zinc-400")
                  }
                  strokeWidth={2}
                />
                {n.label}
              </Link>
            );
          })}
        </nav>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 items-center justify-between border-b border-zinc-200 px-6">
          <h1 className="text-[15px] font-semibold">{title}</h1>
          <UserMenu email={userEmail} />
        </header>
        <main className="flex-1 p-6">
          <div className="mx-auto max-w-4xl">{children}</div>
        </main>
      </div>
    </div>
  );
}

// Avatar + dropdown (email, View site, Sign out). Native <details> so it opens
// on click with no extra state; the marketing nav/footer aren't here, so this
// is the only account control.
function UserMenu({ email }: { email: string }) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="group relative">
      <summary className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-zinc-900 text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden">
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-56 overflow-hidden rounded-xl border border-zinc-200 bg-white py-1 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <div className="border-b border-zinc-100 px-3 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">
            {email || "Signed in"}
          </div>
        </div>
        <a
          href="/"
          className="flex items-center gap-2 px-3 py-2 text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <ExternalLink className="size-4 text-zinc-400" strokeWidth={2} />
          View site
        </a>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <LogOut className="size-4 text-zinc-400" strokeWidth={2} />
          Sign out
        </button>
      </div>
    </details>
  );
}
