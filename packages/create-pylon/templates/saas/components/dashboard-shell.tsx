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
  ChevronsUpDown,
  type LucideIcon,
} from "lucide-react";

export type NavKey = "overview" | "projects" | "members" | "billing" | "settings";

// Exported so `app/dashboard/layout.tsx` can derive the active item + page
// title from the request URL — one table drives both the sidebar and the
// layout's routing awareness. `group` splits the sidebar into sections:
// ungrouped items render first, then each named group under an eyebrow label.
export const NAV: {
  key: NavKey;
  label: string;
  href: string;
  Icon: LucideIcon;
  group?: string;
}[] = [
  { key: "overview", label: "Overview", href: "/dashboard", Icon: LayoutDashboard },
  { key: "projects", label: "Projects", href: "/dashboard/projects", Icon: FolderKanban },
  { key: "members", label: "Members", href: "/dashboard/members", Icon: Users, group: "Workspace" },
  { key: "billing", label: "Billing", href: "/dashboard/billing", Icon: CreditCard, group: "Workspace" },
  { key: "settings", label: "Settings", href: "/dashboard/settings", Icon: SettingsIcon, group: "Workspace" },
];

// Dashboard chrome: a fixed sidebar (logo, workspace switcher, grouped nav,
// account card pinned to the bottom) plus a slim top bar. Rendered by
// `app/dashboard/layout.tsx`, which wraps every /dashboard page — /dashboard
// sits outside the `(marketing)` route group, so this shell is the only chrome
// here. `userEmail` is resolved on the server (serverData.get("User", …)) and
// passed in, so the menu shows a real email instead of a raw id.
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
  const ungrouped = NAV.filter((n) => !n.group);
  const groups = [...new Set(NAV.map((n) => n.group).filter(Boolean))] as string[];
  return (
    <div className="flex min-h-screen bg-white text-zinc-900">
      <aside className="hidden w-60 shrink-0 flex-col border-r border-zinc-200/70 bg-zinc-50/80 md:flex">
        <div className="flex h-14 shrink-0 items-center px-4">
          <Link href="/" className="flex items-center gap-2">
            <span className="flex size-6 items-center justify-center rounded-[7px] bg-zinc-900 text-[13px] font-bold text-white">
              A
            </span>
            <span className="text-[15px] font-semibold tracking-tight">Acme</span>
          </Link>
        </div>

        <div className="px-3 pb-1">
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

        <nav className="flex-1 overflow-y-auto px-3 pt-3">
          <div className="space-y-0.5">
            {ungrouped.map((n) => (
              <NavItem key={n.key} item={n} isActive={active === n.key} />
            ))}
          </div>
          {groups.map((g) => (
            <div key={g} className="mt-6">
              <div className="mb-1.5 px-2.5 text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
                {g}
              </div>
              <div className="space-y-0.5">
                {NAV.filter((n) => n.group === g).map((n) => (
                  <NavItem key={n.key} item={n} isActive={active === n.key} />
                ))}
              </div>
            </div>
          ))}
        </nav>

        {/* Account card pinned to the bottom of the column — the sidebar's
            base holds the session controls instead of trailing off into
            empty space. The dropdown opens UPWARD (bottom-full). */}
        <div className="border-t border-zinc-200/70 p-3">
          <UserMenu email={userEmail} direction="up" />
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between border-b border-zinc-200/70 px-6">
          <h1 className="text-[15px] font-semibold">{title}</h1>
          {/* The sidebar (and its account card) is hidden below md, so the
              top bar carries the account menu on small screens only. */}
          <div className="md:hidden">
            <UserMenu email={userEmail} direction="down" />
          </div>
        </header>
        <main className="flex-1 p-6">
          <div className="mx-auto max-w-4xl">{children}</div>
        </main>
      </div>
    </div>
  );
}

// A single sidebar row. The active row reads as a raised white card — layered
// translucent shadows instead of a hard border, so it sits naturally on the
// zinc-50 column. Inactive rows tint on hover only.
function NavItem({
  item,
  isActive,
}: {
  item: (typeof NAV)[number];
  isActive: boolean;
}) {
  return (
    <Link
      href={item.href}
      className={
        "flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13px] font-medium transition-colors " +
        (isActive
          ? "bg-white text-zinc-900 shadow-[0_1px_2px_rgba(0,0,0,0.06),0_0_0_1px_rgba(0,0,0,0.05)]"
          : "text-zinc-600 hover:bg-zinc-200/50 hover:text-zinc-900")
      }
    >
      <item.Icon
        className={"size-[17px] " + (isActive ? "text-zinc-700" : "text-zinc-400")}
        strokeWidth={2}
      />
      {item.label}
    </Link>
  );
}

// Avatar + dropdown (email, View site, Sign out). Native <details> so it opens
// on click with no extra state. In the sidebar it renders as a full-width
// account card whose menu opens upward; in the mobile top bar it's a compact
// avatar with a downward menu.
function UserMenu({
  email,
  direction,
}: {
  email: string;
  direction: "up" | "down";
}) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  const up = direction === "up";
  return (
    <details className="group relative">
      <summary
        className={
          "cursor-pointer select-none list-none marker:hidden [&::-webkit-details-marker]:hidden " +
          (up
            ? "flex w-full items-center gap-2.5 rounded-lg px-2 py-2 transition-colors hover:bg-zinc-200/50"
            : "flex size-9 items-center justify-center")
        }
      >
        <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-zinc-900 text-[11px] font-semibold text-white">
          {initial}
        </span>
        {up ? (
          <>
            <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-zinc-700">
              {email || "Signed in"}
            </span>
            <ChevronsUpDown className="size-3.5 shrink-0 text-zinc-400" strokeWidth={2} />
          </>
        ) : null}
      </summary>
      <div
        className={
          "absolute z-40 w-56 overflow-hidden rounded-xl bg-white p-1 shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_16px_48px_-16px_rgba(0,0,0,0.25)] " +
          (up ? "bottom-full left-0 mb-2" : "right-0 top-full mt-2")
        }
      >
        <div className="border-b border-zinc-100 px-2.5 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">
            {email || "Signed in"}
          </div>
        </div>
        <a
          href="/"
          className="flex items-center gap-2 rounded-lg px-2.5 py-2 text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <ExternalLink className="size-4 text-zinc-400" strokeWidth={2} />
          View site
        </a>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <LogOut className="size-4 text-zinc-400" strokeWidth={2} />
          Sign out
        </button>
      </div>
    </details>
  );
}
