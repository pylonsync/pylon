import { useEffect, useRef, useState } from "react";
import { LogOut, User as UserIcon, Wallet } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import type { AuthUser } from "./lib/types";
import { logout } from "./lib/auth";
import { formatCents, initials, navigate } from "./lib/util";

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

  const label = user.displayName ?? user.email ?? "Account";
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
        <div className="absolute right-0 top-full mt-1 w-60 rounded-md border bg-popover p-1 shadow-md">
          <div className="border-b p-3">
            <div className="text-sm font-medium">{label}</div>
            {user.email && (
              <div className="truncate text-xs text-muted-foreground">
                {user.email}
              </div>
            )}
            {user.balanceCents != null && (
              <div className="mt-2 flex items-center gap-1.5 text-xs text-muted-foreground">
                <Wallet className="size-3" />
                <span className="font-mono">{formatCents(user.balanceCents)}</span>
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
            My bids
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
