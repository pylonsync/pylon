"use client";
/**
 * Gate for routes that need a real (non-guest) account — /account, /checkout.
 * Guests browsing anonymously hit this instead of a half-broken page; the
 * buttons open the shared auth dialog (hosted in <StoreChrome>).
 */
import React from "react";
import { User as UserIcon } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { useAuth } from "./lib/auth";
import { openAuth } from "./lib/util";

export function RequireAuth({ children }: { children: React.ReactNode }) {
  const { isAuthenticated } = useAuth();

  if (isAuthenticated) return <>{children}</>;

  return (
    <main className="mx-auto flex max-w-md flex-col items-center gap-4 p-8 text-center">
      <UserIcon className="size-12 text-muted-foreground" />
      <h2 className="text-xl font-semibold">Sign in to continue</h2>
      <p className="text-sm text-muted-foreground">
        Your cart, orders, and shipping details live with your account. Create
        one in 10 seconds — no email verification required for the demo.
      </p>
      <div className="flex w-full gap-2">
        <Button className="flex-1" onClick={() => openAuth("register")}>
          Sign up
        </Button>
        <Button
          className="flex-1"
          variant="outline"
          onClick={() => openAuth("login")}
        >
          Log in
        </Button>
      </div>
    </main>
  );
}
