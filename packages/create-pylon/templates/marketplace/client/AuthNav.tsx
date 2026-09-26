"use client";

import React from "react";
import { Link } from "@pylonsync/react";
import { MarketProvider, useAuth } from "./MarketProvider";
import { avatarColor, initials } from "./market";

// Sign-in state for the header. Signed out: a link to the dashboard, which
// shows the prefilled demo sign-in. Signed in: an avatar and a sign-out button.
function Nav() {
  const { identity, signOut } = useAuth();
  if (!identity) {
    return (
      <Link
        href="/me"
        className="inline-flex min-h-10 items-center rounded-full px-3.5 text-sm font-medium shadow-[var(--shadow-border)] transition-shadow hover:shadow-[var(--shadow-border-hover)]"
      >
        Sign in
      </Link>
    );
  }
  return (
    <div className="flex items-center gap-1">
      <Link
        href="/me"
        aria-label={`Your market (${identity.name})`}
        title={identity.name}
        className="grid size-9 place-items-center rounded-full text-xs font-semibold text-white"
        style={{ background: avatarColor(identity.name) }}
      >
        {initials(identity.name)}
      </Link>
      <button
        type="button"
        onClick={() => void signOut()}
        className="hidden min-h-10 items-center rounded-full px-3 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground lg:inline-flex"
      >
        Sign out
      </button>
    </div>
  );
}

export function AuthNav() {
  return (
    <MarketProvider fallback={<span className="inline-block w-[72px]" />}>
      <Nav />
    </MarketProvider>
  );
}
