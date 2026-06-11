"use client";
/**
 * User menu — avatar button + dropdown for account / orders / sign-out.
 * Uses @pylonsync/example-ui/dropdown-menu (Radix-backed) for proper
 * keyboard navigation and focus trapping.
 */
import { LogOut, Package, User as UserIcon } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@pylonsync/example-ui/dropdown-menu";
import { Button } from "@pylonsync/example-ui/button";
import type { AuthUser } from "./lib/types";
import { logout } from "./lib/auth";
import { navigate } from "./lib/util";

export function UserMenu({ user }: { user: NonNullable<AuthUser> }) {
  const initials =
    (user.name ?? user.email ?? "U")
      .split(/\s|@/)
      .filter(Boolean)
      .slice(0, 2)
      .map((p) => p[0]?.toUpperCase() ?? "")
      .join("") || "U";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          className="flex items-center gap-2 rounded-full border bg-background px-2 py-1 text-sm hover:bg-accent h-auto"
        >
          <span className="flex size-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
            {initials}
          </span>
          <span className="hidden text-foreground/80 sm:inline">
            {user.name ?? user.email ?? "Account"}
          </span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuLabel className="flex flex-col gap-0.5">
          <span className="font-medium">{user.name ?? "Customer"}</span>
          {user.email && (
            <span className="truncate text-xs font-normal text-muted-foreground">
              {user.email}
            </span>
          )}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={() => navigate("#/account")}>
          <UserIcon className="size-4" />
          Account
        </DropdownMenuItem>
        <DropdownMenuItem onClick={() => navigate("#/account")}>
          <Package className="size-4" />
          Orders
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
