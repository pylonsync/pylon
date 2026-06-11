"use client";

import { LogOut, User as UserIcon, Wallet } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@pylonsync/example-ui/dropdown-menu";
import type { AuthUser } from "./lib/types";
import { logout } from "./lib/auth";
import { formatCents, initials, navigate } from "./lib/util";

export function UserMenu({ user }: { user: NonNullable<AuthUser> }) {
  const label = user.displayName ?? user.email ?? "Account";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button className="flex items-center gap-2 rounded-full border bg-background px-2 py-1 text-sm hover:bg-accent">
          <span className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
            {initials(label)}
          </span>
          <span className="hidden text-foreground/80 sm:inline">{label}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        <DropdownMenuLabel className="font-normal">
          <div className="text-sm font-medium">{label}</div>
          {user.email && (
            <div className="truncate text-xs text-muted-foreground">
              {user.email}
            </div>
          )}
          {user.balanceCents != null && (
            <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground">
              <Wallet className="size-3" />
              <span className="font-mono tabular-nums">
                {formatCents(user.balanceCents)}
              </span>
            </div>
          )}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onClick={() => navigate("#/account")}
        >
          <UserIcon className="size-4" />
          My bids
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          onClick={async () => {
            await logout();
            navigate("#/");
          }}
        >
          <LogOut className="size-4" />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
