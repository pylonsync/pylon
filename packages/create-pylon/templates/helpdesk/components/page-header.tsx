"use client";

import React from "react";
import { Menu, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { useShell } from "@/components/app-shell";

/**
 * The bar at the top of every view: title on the left, actions on the right.
 * Fixed height, so switching pages doesn't shift the content below it.
 *
 * Below the `md` breakpoint it also carries the menu button that opens the
 * navigation drawer and a search button, since the sidebar is hidden there.
 */
export function PageHeader({
  title,
  count,
  leading,
  children,
  className,
}: {
  title: string;
  /** Shown beside the title. Pass the same number the sidebar shows. */
  count?: number;
  /** Rendered before the title, such as a back link on a detail page. */
  leading?: React.ReactNode;
  children?: React.ReactNode;
  className?: string;
}) {
  const shell = useShell();
  return (
    <header
      className={cn(
        "flex h-13 shrink-0 items-center justify-between gap-2 border-b border-border px-3 md:gap-3 md:px-5",
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        {shell ? (
          <button
            type="button"
            onClick={shell.openNav}
            aria-label="Open navigation"
            className="-ml-1 rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-foreground/[0.05] hover:text-foreground md:hidden"
          >
            <Menu className="size-[18px]" />
          </button>
        ) : null}
        {leading}
        <h1 className="truncate text-[14px] font-semibold tracking-[-0.01em]">{title}</h1>
        {typeof count === "number" ? (
          <span className="tabular ml-0.5 rounded-full bg-foreground/[0.06] px-1.5 py-px text-[11.5px] font-medium text-muted-foreground">
            {count}
          </span>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1.5 md:gap-2">
        {shell ? (
          <button
            type="button"
            onClick={shell.openCommand}
            aria-label="Search"
            className="rounded-md p-1.5 text-muted-foreground transition-colors hover:bg-foreground/[0.05] hover:text-foreground md:hidden"
          >
            <Search className="size-[18px]" />
          </button>
        ) : null}
        {children}
      </div>
    </header>
  );
}
