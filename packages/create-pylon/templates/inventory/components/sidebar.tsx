import React from "react";
import { Link } from "@pylonsync/react";
import { LogOut, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { BRAND } from "@/lib/brand";
import { NAV } from "@/components/nav";
import { Avatar } from "@/components/avatar";
import { BrandMark } from "@/components/brand-mark";
import { Kbd } from "@/components/kbd";

/** True when `href` is the section `pathname` belongs to. */
export function isActive(href: string, pathname: string): boolean {
  if (href === "/") return pathname === "/";
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * The navigation rail: brand, search, links with counts, and the signed-in
 * person. The same component renders as the fixed rail on wide screens and
 * inside the drawer on phones (see components/app-shell.tsx).
 *
 * Presentational: the path, counts, and handlers arrive as props, so it
 * renders the same in a test.
 */
export function Sidebar({
  user,
  pathname,
  counts,
  onOpenCommand,
  onNavigate,
  onSignOut,
  className,
}: {
  user: { name: string; email: string };
  pathname: string;
  /** Badge per nav href. Omit a key to show no number. */
  counts?: Record<string, number>;
  onOpenCommand: () => void;
  /** Called after a link is chosen, so the phone drawer can close. */
  onNavigate?: () => void;
  onSignOut: () => void;
  className?: string;
}) {
  return (
    <div className={cn("flex h-full w-full flex-col bg-surface-1", className)}>
      <div className="flex h-13 shrink-0 items-center gap-2.5 px-4">
        <BrandMark />
        <span className="truncate text-[14px] font-semibold tracking-[-0.01em]">
          {BRAND.name}
        </span>
      </div>

      <div className="px-3 pb-3">
        <button
          type="button"
          onClick={onOpenCommand}
          className={cn(
            "flex h-8 w-full items-center gap-2 rounded-md border border-border bg-background px-2.5",
            "text-[12.5px] text-muted-foreground shadow-[0_1px_1px_rgb(0_0_0/0.03)] transition-colors",
            "hover:border-foreground/15 hover:text-foreground",
          )}
        >
          <Search className="size-3.5" />
          <span className="flex-1 text-left">Search</span>
          <Kbd className="max-md:hidden">⌘K</Kbd>
        </button>
      </div>

      <nav className="flex-1 space-y-px overflow-y-auto px-3" aria-label="Main">
        {NAV.map((item) => {
          const active = isActive(item.href, pathname);
          const count = counts?.[item.href];
          return (
            <Link
              key={item.href}
              href={item.href}
              onClick={onNavigate}
              aria-current={active ? "page" : undefined}
              className={cn(
                "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors md:h-7.5",
                "[&_svg]:size-4 [&_svg]:shrink-0",
                active
                  ? "bg-foreground/[0.06] font-medium text-foreground [&_svg]:text-primary"
                  : "text-muted-foreground hover:bg-foreground/[0.04] hover:text-foreground",
              )}
            >
              {item.icon}
              <span className="flex-1 truncate">{item.label}</span>
              {typeof count === "number" ? (
                <span className="tabular text-[11.5px] text-muted-foreground">{count}</span>
              ) : null}
            </Link>
          );
        })}
      </nav>

      <div className="border-t border-border p-3">
        <div className="flex items-center gap-2.5">
          <Avatar name={user.name || user.email} />
          <div className="min-w-0 flex-1 leading-tight">
            <p className="truncate text-[12.5px] font-medium">{user.name || "Signed in"}</p>
            <p className="truncate text-[11.5px] text-muted-foreground">{user.email}</p>
          </div>
          <button
            type="button"
            onClick={onSignOut}
            title="Sign out"
            aria-label="Sign out"
            className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-foreground/[0.06] hover:text-foreground"
          >
            <LogOut className="size-4" />
          </button>
        </div>
      </div>
    </div>
  );
}
