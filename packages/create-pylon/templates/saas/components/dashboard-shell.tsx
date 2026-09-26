"use client";

import React, { useState } from "react";
import { Link, useRouter } from "@pylonsync/react";
import { useAuth, OrganizationSwitcher } from "@pylonsync/client";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import {
  LayoutGrid,
  FolderKanban,
  Users,
  CreditCard,
  Settings as SettingsIcon,
  LogOut,
  ExternalLink,
  ChevronsUpDown,
  ChevronRight,
  Menu,
  X,
  type LucideIcon,
} from "lucide-react";
import { Avatar, ProjectMark } from "@/components/task-ui";
import { useLiveRows } from "@/lib/use-live";
import { siteConfig } from "@/lib/site.config";

export type NavKey = "overview" | "projects" | "members" | "billing" | "settings";

// Exported so `app/dashboard/layout.tsx` can derive the active item and the
// page title from the request URL. One table drives both the sidebar and the
// layout. `group: "workspace"` items render in the lower block.
export const NAV: {
  key: NavKey;
  label: string;
  href: string;
  Icon: LucideIcon;
  group?: "workspace";
}[] = [
  { key: "overview", label: "Overview", href: "/dashboard", Icon: LayoutGrid },
  { key: "projects", label: "Projects", href: "/dashboard/projects", Icon: FolderKanban },
  { key: "members", label: "Members", href: "/dashboard/members", Icon: Users, group: "workspace" },
  { key: "billing", label: "Billing", href: "/dashboard/billing", Icon: CreditCard, group: "workspace" },
  { key: "settings", label: "Settings", href: "/dashboard/settings", Icon: SettingsIcon, group: "workspace" },
];

export interface ShellProject {
  id: string;
  orgId: string;
  name: string;
  status?: string;
}

// Dashboard chrome. From md up: a sidebar on a grey frame with the page in a
// raised white panel. Below md: a top bar whose menu button opens the same
// sidebar as a drawer. Rendered by `app/dashboard/layout.tsx`, which resolves
// every prop on the server, so the first paint has the real names.
export function DashboardShell({
  active,
  title,
  parent,
  activeProjectId,
  tenantId,
  userId,
  userName,
  userEmail,
  orgName,
  projects,
  children,
}: {
  active: NavKey;
  title: string;
  /** Breadcrumb parent, e.g. Projects on a project's board. */
  parent?: { label: string; href: string };
  activeProjectId?: string;
  tenantId: string;
  userId: string;
  userName: string;
  userEmail: string;
  orgName?: string;
  projects: ShellProject[];
  children: React.ReactNode;
}) {
  const [drawer, setDrawer] = useState(false);
  const sidebar = (
    <Sidebar
      active={active}
      activeProjectId={activeProjectId}
      tenantId={tenantId}
      userId={userId}
      userName={userName}
      userEmail={userEmail}
      orgName={orgName}
      projects={projects}
      onNavigate={() => setDrawer(false)}
    />
  );
  return (
    <div className="min-h-screen bg-[#f4f4f5] text-zinc-900 md:flex">
      <aside className="sticky top-0 hidden h-screen w-[244px] shrink-0 md:block">{sidebar}</aside>

      {/* Mobile top bar + drawer. */}
      <DialogPrimitive.Root open={drawer} onOpenChange={setDrawer}>
        <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b border-zinc-200/80 bg-white/90 px-3 backdrop-blur md:hidden">
          <DialogPrimitive.Trigger
            aria-label="Open navigation"
            className="flex size-9 items-center justify-center rounded-lg text-zinc-600 transition-colors hover:bg-zinc-100"
          >
            <Menu className="size-[18px]" strokeWidth={1.75} />
          </DialogPrimitive.Trigger>
          <div className="min-w-0 flex-1 truncate text-[14px] font-medium">{title}</div>
          <Avatar id={userId} name={userName} />
        </header>
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-zinc-950/30 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 md:hidden" />
          <DialogPrimitive.Content className="fixed inset-y-0 left-0 z-50 w-[280px] max-w-[85vw] bg-[#f4f4f5] shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_24px_64px_-12px_rgba(0,0,0,0.3)] data-[state=open]:animate-in data-[state=open]:slide-in-from-left data-[state=closed]:animate-out data-[state=closed]:slide-out-to-left md:hidden">
            <DialogPrimitive.Title className="sr-only">Navigation</DialogPrimitive.Title>
            <DialogPrimitive.Description className="sr-only">
              Pages and projects in this workspace
            </DialogPrimitive.Description>
            <DialogPrimitive.Close
              aria-label="Close navigation"
              className="absolute right-2.5 top-3 z-10 flex size-8 items-center justify-center rounded-lg text-zinc-500 hover:bg-zinc-200/60"
            >
              <X className="size-4" />
            </DialogPrimitive.Close>
            {sidebar}
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>

      <div className="min-w-0 flex-1 md:py-2 md:pr-2">
        <div className="flex min-h-screen flex-col bg-white md:min-h-[calc(100vh-1rem)] md:rounded-xl md:shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_1px_2px_rgba(0,0,0,0.04)]">
          <div className="hidden h-12 shrink-0 items-center gap-1.5 border-b border-zinc-100 px-6 text-[13px] md:flex">
            {parent ? (
              <>
                <Link href={parent.href} className="text-zinc-500 transition-colors hover:text-zinc-900">
                  {parent.label}
                </Link>
                <ChevronRight className="size-3.5 text-zinc-300" />
              </>
            ) : null}
            <span className="truncate font-medium text-zinc-900">{title}</span>
          </div>
          <main className="min-w-0 flex-1 px-4 py-6 sm:px-6 md:px-8 md:py-8">{children}</main>
        </div>
      </div>
    </div>
  );
}

function Sidebar({
  active,
  activeProjectId,
  tenantId,
  userId,
  userName,
  userEmail,
  orgName,
  projects,
  onNavigate,
}: {
  active: NavKey;
  activeProjectId?: string;
  tenantId: string;
  userId: string;
  userName: string;
  userEmail: string;
  orgName?: string;
  projects: ShellProject[];
  onNavigate: () => void;
}) {
  const router = useRouter();
  const live = useLiveRows<ShellProject>("Project", projects, tenantId)
    .filter((p) => (p.status ?? "active") !== "archived")
    .sort((a, b) => a.name.localeCompare(b.name, "en"));
  const main = NAV.filter((n) => !n.group);
  const workspace = NAV.filter((n) => n.group === "workspace");
  return (
    <div className="flex h-full flex-col px-3 pb-3">
      <div className="flex h-14 shrink-0 items-center gap-2 px-2">
        <Link href="/" className="flex items-center gap-2" onClick={onNavigate}>
          <span className="flex size-[22px] items-center justify-center rounded-[6px] bg-zinc-900 text-[12px] font-bold text-white">
            {siteConfig.brand.letter}
          </span>
          <span className="text-[14px] font-semibold tracking-tight">{siteConfig.brand.name}</span>
        </Link>
      </div>

      {/* Switching workspaces re-renders the dashboard for the new tenant.
          router.push re-fetches the SSR page without a full reload. */}
      <div className="[&>div]:w-full [&_button[aria-haspopup]]:w-full [&_button[aria-haspopup]]:justify-start [&_button[aria-haspopup]]:border-zinc-200/80 [&_button[aria-haspopup]]:bg-white [&_button[aria-haspopup]]:px-2 [&_button[aria-haspopup]]:py-1.5 [&_button[aria-haspopup]]:text-[13px] [&_button[aria-haspopup]]:shadow-[0_1px_1px_rgba(0,0,0,0.03)] [&_button[aria-haspopup]>span:nth-child(2)]:flex-1 [&_button[aria-haspopup]>span:nth-child(2)]:text-left">
        <OrganizationSwitcher
          hidePersonal
          initialActiveName={orgName}
          onSwitched={() => {
            onNavigate();
            router.push("/dashboard");
          }}
        />
      </div>

      <nav className="mt-4 flex-1 overflow-y-auto">
        <div className="space-y-px">
          {main.map((n) => (
            <NavItem key={n.key} item={n} isActive={active === n.key && !activeProjectId} onNavigate={onNavigate} />
          ))}
        </div>

        {live.length > 0 ? (
          <div className="mt-5">
            <div className="px-2 pb-1 text-[12px] font-medium text-zinc-400">Active projects</div>
            <div className="space-y-px">
              {live.map((p) => (
                <Link
                  key={p.id}
                  href={`/dashboard/projects/${p.id}`}
                  onClick={onNavigate}
                  className={
                    "flex items-center gap-2 rounded-md px-2 py-[5px] text-[13px] transition-colors " +
                    (activeProjectId === p.id
                      ? "bg-white font-medium text-zinc-900 shadow-[0_0_0_1px_rgba(0,0,0,0.05),0_1px_2px_rgba(0,0,0,0.05)]"
                      : "text-zinc-600 hover:bg-zinc-200/50 hover:text-zinc-900")
                  }
                >
                  <ProjectMark id={p.id} name={p.name} className="size-[18px] text-[8.5px]" />
                  <span className="truncate">{p.name}</span>
                </Link>
              ))}
            </div>
          </div>
        ) : null}

        <div className="mt-5">
          <div className="px-2 pb-1 text-[12px] font-medium text-zinc-400">Workspace</div>
          <div className="space-y-px">
            {workspace.map((n) => (
              <NavItem key={n.key} item={n} isActive={active === n.key} onNavigate={onNavigate} />
            ))}
          </div>
        </div>
      </nav>

      <UserMenu userId={userId} name={userName} email={userEmail} />
    </div>
  );
}

function NavItem({
  item,
  isActive,
  onNavigate,
}: {
  item: (typeof NAV)[number];
  isActive: boolean;
  onNavigate: () => void;
}) {
  return (
    <Link
      href={item.href}
      onClick={onNavigate}
      className={
        "flex items-center gap-2.5 rounded-md px-2 py-[6px] text-[13px] transition-colors " +
        (isActive
          ? "bg-white font-medium text-zinc-900 shadow-[0_0_0_1px_rgba(0,0,0,0.05),0_1px_2px_rgba(0,0,0,0.05)]"
          : "text-zinc-600 hover:bg-zinc-200/50 hover:text-zinc-900")
      }
    >
      <item.Icon className={"size-4 " + (isActive ? "text-zinc-800" : "text-zinc-400")} strokeWidth={1.75} />
      {item.label}
    </Link>
  );
}

// Account card pinned to the bottom of the sidebar. Native <details>, so it
// opens on click with no extra state; the menu opens upward.
function UserMenu({ userId, name, email }: { userId: string; name: string; email: string }) {
  const { signOut } = useAuth();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="relative">
      <summary className="flex cursor-pointer select-none list-none items-center gap-2.5 rounded-lg px-2 py-2 transition-colors marker:hidden hover:bg-zinc-200/50 [&::-webkit-details-marker]:hidden">
        <Avatar id={userId} name={name} />
        <span className="min-w-0 flex-1 leading-tight">
          <span className="block truncate text-[13px] font-medium text-zinc-800">{name}</span>
          <span className="block truncate text-[11.5px] text-zinc-500">{email}</span>
        </span>
        <ChevronsUpDown className="size-3.5 shrink-0 text-zinc-400" strokeWidth={1.75} />
      </summary>
      <div className="absolute bottom-full left-0 z-40 mb-2 w-full min-w-52 overflow-hidden rounded-xl bg-white p-1 shadow-[0_0_0_1px_rgba(0,0,0,0.06),0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <a
          href="/"
          className="flex items-center gap-2 rounded-lg px-2.5 py-2 text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <ExternalLink className="size-4 text-zinc-400" strokeWidth={1.75} />
          View site
        </a>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          <LogOut className="size-4 text-zinc-400" strokeWidth={1.75} />
          Sign out
        </button>
      </div>
    </details>
  );
}
