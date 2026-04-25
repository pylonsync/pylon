/**
 * Tiny user menu — avatar circle + dropdown for account / orders /
 * logout. Uses a simple click-outside hook instead of Radix because
 * a native <details> element gives us 99% of what we need with zero
 * accessibility traps.
 */
import { useEffect, useRef, useState } from "react";
import { LogOut, Package, User as UserIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { AuthUser } from "./lib/types";
import { logout } from "./lib/auth";
import { navigate } from "./lib/util";

export function UserMenu({ user }: { user: NonNullable<AuthUser> }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onClick);
    return () => window.removeEventListener("mousedown", onClick);
  }, [open]);

  const initials =
    (user.name ?? user.email ?? "U")
      .split(/\s|@/)
      .filter(Boolean)
      .slice(0, 2)
      .map((p) => p[0]?.toUpperCase() ?? "")
      .join("") || "U";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-2 rounded-full border bg-background px-2 py-1 text-sm hover:bg-accent"
      >
        <span className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
          {initials}
        </span>
        <span className="hidden text-foreground/80 sm:inline">
          {user.name ?? user.email ?? "Account"}
        </span>
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
            onClick={() => {
              navigate("#/account");
              setOpen(false);
            }}
          >
            <UserIcon className="size-4" />
            Account
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start"
            onClick={() => {
              navigate("#/account");
              setOpen(false);
            }}
          >
            <Package className="size-4" />
            Orders
          </Button>
          <Button
            variant="ghost"
            className="w-full justify-start text-destructive hover:text-destructive"
            onClick={async () => {
              await logout();
              setOpen(false);
              navigate("#/");
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
