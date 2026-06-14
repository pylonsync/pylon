"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { useAuth, OrganizationSwitcher } from "@pylonsync/client";

type NavKey = "overview" | "projects" | "members" | "settings";

const NAV: { key: NavKey; label: string; href: string; icon: string }[] = [
  { key: "overview", label: "Overview", href: "/dashboard", icon: "▦" },
  { key: "projects", label: "Projects", href: "/dashboard/projects", icon: "▤" },
  { key: "members", label: "Members", href: "/dashboard/members", icon: "◍" },
  { key: "settings", label: "Settings", href: "/dashboard/settings", icon: "⚙" },
];

// Dashboard chrome: a fixed sidebar (logo, workspace switcher, nav, sign-out)
// plus a top bar and the page content. The marketing nav/footer are suppressed
// for /dashboard in the root layout, so this shell is the only chrome here.
// Each dashboard route passes its `active` key + `title`.
export function DashboardShell({
  active,
  title,
  children,
}: {
  active: NavKey;
  title: string;
  children: React.ReactNode;
}) {
  const { signOut } = useAuth();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }

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
          <OrganizationSwitcher />
        </div>

        <nav className="flex-1 space-y-0.5 px-3">
          {NAV.map((n) => (
            <Link
              key={n.key}
              href={n.href}
              className={
                "flex items-center gap-2.5 rounded-md px-3 py-2 text-[13.5px] font-medium transition-colors " +
                (active === n.key
                  ? "bg-zinc-900 text-white"
                  : "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900")
              }
            >
              <span className="w-4 text-center text-[12px] opacity-80">
                {n.icon}
              </span>
              {n.label}
            </Link>
          ))}
        </nav>

        <div className="border-t border-zinc-200 p-3">
          <button
            type="button"
            onClick={onSignOut}
            className="flex w-full items-center gap-2.5 rounded-md px-3 py-2 text-[13.5px] font-medium text-zinc-600 transition-colors hover:bg-zinc-100 hover:text-zinc-900"
          >
            <span className="w-4 text-center text-[12px] opacity-80">⏻</span>
            Sign out
          </button>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 items-center justify-between border-b border-zinc-200 px-6">
          <h1 className="text-[15px] font-semibold">{title}</h1>
          <Link
            href="/"
            className="text-[13px] text-zinc-500 transition-colors hover:text-zinc-900"
          >
            View site →
          </Link>
        </header>
        <main className="flex-1 p-6">
          <div className="mx-auto max-w-4xl">{children}</div>
        </main>
      </div>
    </div>
  );
}
