"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import { MarketProvider, useAuth } from "./MarketProvider";

// Compact sign-in state for the header. Signed out → a link to /sell (which
// gates with the prefilled demo login). Signed in → your name + a sign-out
// button. Lives behind MarketProvider like every other island.
function Nav() {
  const { identity, signOut } = useAuth();
  if (!identity) {
    return (
      <Button asChild variant="outline" size="sm">
        <Link href="/sell">Sign in</Link>
      </Button>
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
