import React from "react";
import { Link } from "@pylonsync/react";
import { Command, Inbox, LogOut, Users } from "lucide-react";
import { cn } from "@/lib/utils";
import { Avatar } from "@/components/avatar";
import { Kbd } from "@/components/kbd";

export interface NavItem {
  href: string;
  label: string;
  icon: React.ReactNode;
  count?: number;
}

export const NAV: Array<Omit<NavItem, "count">> = [
  { href: "/", label: "Inbox", icon: <Inbox /> },
  { href: "/customers", label: "Customers", icon: <Users /> },
];

/**
 * The fixed left rail. Presentational: the active path, the counts, and the
 * sign-out handler all arrive as props, so it renders identically in a test.
 */
export function Sidebar({
  workspace,
  email,
  pathname,
  counts,
  onOpenCommand,
  onSignOut,
}: {
  workspace: string;
  email: string;
  pathname: string;
  counts?: Record<string, number>;
  onOpenCommand: () => void;
  onSignOut: () => void;
}) {
  return (
    <aside className="flex w-[224px] shrink-0 flex-col border-r border-border bg-surface-1">
      <div className="flex h-12 items-center gap-2 px-3">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-primary text-[11px] font-bold text-primary-foreground">
          {workspace.slice(0, 1).toUpperCase()}
        </span>
        <span className="truncate text-[13px] font-semibold">{workspace}</span>
      </div>

      <div className="px-2 pb-2">
        <button
          type="button"
          onClick={onOpenCommand}
          className={cn(
            "flex w-full items-center gap-2 rounded-md border border-border bg-background px-2 py-1.5",
            "text-[12px] text-muted-foreground transition-colors hover:border-ring/40 hover:text-foreground",
          )}
        >
          <Command className="size-3.5" />
          <span className="flex-1 text-left">Search…</span>
          <Kbd>⌘K</Kbd>
        </button>
      </div>

      <nav className="flex-1 space-y-0.5 px-2" aria-label="Main">
        {NAV.map((item) => {
          const active =
            item.href === "/" ? pathname === "/" : pathname.startsWith(item.href);
          return (
            <Link
              key={item.href}
              href={item.href}
              aria-current={active ? "page" : undefined}
              className={cn(
                "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors",
                "[&_svg]:size-3.5 [&_svg]:shrink-0",
                active
                  ? "bg-surface-2 font-medium text-foreground"
                  : "text-muted-foreground hover:bg-surface-2/60 hover:text-foreground",
              )}
            >
              {item.icon}
              <span className="flex-1 truncate">{item.label}</span>
              {counts?.[item.href] ? (
                <span className="tabular text-[11px] text-muted-foreground">
                  {counts[item.href]}
                </span>
              ) : null}
            </Link>
          );
        })}
      </nav>

      <div className="border-t border-border p-2">
        <div className="flex items-center gap-2 rounded-md px-1.5 py-1">
          <Avatar name={email} size="sm" />
          <span className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
            {email}
          </span>
          <button
            type="button"
            onClick={onSignOut}
            title="Sign out"
            aria-label="Sign out"
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-surface-2 hover:text-foreground"
          >
            <LogOut className="size-3.5" />
          </button>
        </div>
      </div>
    </aside>
  );
}
