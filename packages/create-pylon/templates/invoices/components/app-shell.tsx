"use client";

import React, { createContext, useContext, useEffect, useState } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { BRAND } from "@/lib/brand";
import { Sidebar } from "@/components/sidebar";

interface ShellControls {
  /** Opens the navigation drawer. Only reachable on narrow screens. */
  openNav: () => void;
  openCommand: () => void;
}

const ShellContext = createContext<ShellControls | null>(null);

/** The shell's controls, or null outside a shell (a component test). */
export function useShell(): ShellControls | null {
  return useContext(ShellContext);
}

type SidebarProps = React.ComponentProps<typeof Sidebar>;

/**
 * The page frame. From the `md` breakpoint up, the sidebar is a fixed
 * 240px rail and the main column scrolls on its own. Below it, the rail
 * is hidden and the same sidebar opens as a drawer from the menu button
 * in each page header, so a phone gets the full width for content.
 */
export function AppShell({
  sidebar,
  children,
}: {
  sidebar: Omit<SidebarProps, "onNavigate" | "className">;
  children: React.ReactNode;
}) {
  const [navOpen, setNavOpen] = useState(false);
  const { pathname, onOpenCommand } = sidebar;

  // A route change closes the drawer, including one made from ⌘K.
  useEffect(() => {
    setNavOpen(false);
  }, [pathname]);

  // Widening past md hides the drawer, so close it rather than leave an
  // invisible modal holding focus.
  useEffect(() => {
    const wide = window.matchMedia("(min-width: 768px)");
    const onChange = () => {
      if (wide.matches) setNavOpen(false);
    };
    wide.addEventListener("change", onChange);
    return () => wide.removeEventListener("change", onChange);
  }, []);

  return (
    <ShellContext.Provider
      value={{ openNav: () => setNavOpen(true), openCommand: onOpenCommand }}
    >
      <div className="flex h-dvh overflow-hidden bg-background">
        <aside className="hidden w-[240px] shrink-0 border-r border-border md:flex">
          <Sidebar {...sidebar} />
        </aside>

        <DialogPrimitive.Root open={navOpen} onOpenChange={setNavOpen}>
          <DialogPrimitive.Portal>
            <DialogPrimitive.Overlay className="fixed inset-0 z-40 bg-black/40 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 md:hidden" />
            <DialogPrimitive.Content
              className={
                "fixed inset-y-0 left-0 z-40 flex w-[min(300px,86vw)] border-r border-border shadow-xl outline-none md:hidden " +
                "duration-200 data-[state=open]:animate-in data-[state=open]:slide-in-from-left " +
                "data-[state=closed]:animate-out data-[state=closed]:slide-out-to-left"
              }
            >
              <DialogPrimitive.Title className="sr-only">{BRAND.name} navigation</DialogPrimitive.Title>
              <DialogPrimitive.Description className="sr-only">
                Links to every section, search, and your account.
              </DialogPrimitive.Description>
              <Sidebar
                {...sidebar}
                onNavigate={() => setNavOpen(false)}
                onOpenCommand={() => {
                  setNavOpen(false);
                  onOpenCommand();
                }}
              />
            </DialogPrimitive.Content>
          </DialogPrimitive.Portal>
        </DialogPrimitive.Root>

        <main className="flex min-w-0 flex-1 flex-col">{children}</main>
      </div>
    </ShellContext.Provider>
  );
}
