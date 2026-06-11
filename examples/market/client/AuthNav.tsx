"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { Button } from "@pylonsync/example-ui/button";
import { MarketProvider, useAuth } from "./MarketProvider";

// Compact sign-in state for the header. Signed out → a link to /sell (which
// gates with the prefilled demo login). Signed in → your name + a sign-out
// button. Lives behind MarketProvider like every other island.
function Nav() {
  const { identity, signOut } = useAuth();
  if (!identity) {
    return (
      <Link
        href="/sell"
        className="rounded-md border px-3 py-1.5 text-sm font-medium transition hover:bg-muted"
      >
        Sign in
      </Link>
    );
  }
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="hidden text-muted-foreground sm:inline">
        {identity.name}
      </span>
      <Button
        variant="ghost"
        size="sm"
        className="text-muted-foreground"
        onClick={() => void signOut()}
      >
        Sign out
      </Button>
    </div>
  );
}

export function AuthNav() {
  return (
    <MarketProvider fallback={<span className="w-16" />}>
      <Nav />
    </MarketProvider>
  );
}
