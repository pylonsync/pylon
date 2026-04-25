"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { LogOut, Package, User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { logout, type AuthUser } from "@/lib/pylon-client";
import { initials } from "@/lib/util";

export function UserMenu({ user }: { user: NonNullable<AuthUser> }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const router = useRouter();

  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onClick);
    return () => window.removeEventListener("mousedown", onClick);
  }, [open]);

  const label = user.name ?? user.email ?? "Account";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-2 rounded-full border bg-background px-2 py-1 text-sm hover:bg-accent"
      >
        <span className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
          {initials(label)}
        </span>
        <span className="hidden text-foreground/80 sm:inline">{label}</span>
      </button>
      {open && (
        <div className="absolute right-0 top-full mt-1 w-56 rounded-md border bg-popover p-1 shadow-md">
          <div className="border-b p-3">
            <div className="text-sm font-medium">{user.name ?? "Customer"}</div>
            {user.email && (
              <div className="truncate text-xs text-muted-foreground">
                {user.email}
              </div>
            )}
          </div>
          <Button
            variant="ghost"
            className="w-full justify-start"
            asChild
            onClick={() => setOpen(false)}
          >
            <Link href="/account">
              <UserIcon className="size-4" />
              Account
            </Link>
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start"
            asChild
            onClick={() => setOpen(false)}
          >
            <Link href="/account">
              <Package className="size-4" />
              Orders
            </Link>
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start text-destructive hover:text-destructive"
            onClick={async () => {
              await logout();
              setOpen(false);
              router.push("/");
              router.refresh();
            }}
          >
            <LogOut className="size-4" />
            Sign out
          </Button>
        </div>
      )}
    </div>
  );
}
