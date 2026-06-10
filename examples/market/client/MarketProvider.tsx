"use client";

import React, {
  createContext,
  useContext,
  useEffect,
  useState,
} from "react";
import { ensureIdentity, type Identity } from "./market";

// The sync engine (live queries, WebSocket) is browser-only, so every
// interactive island mounts behind this provider: it renders a fallback
// during SSR + before the guest session lands, then exposes the identity to
// children once the engine is booted. Establishing the session BEFORE
// rendering children is load-bearing — the first mutation (offer/listing)
// needs a signed-in caller.
const IdentityContext = createContext<Identity | null>(null);

export function useIdentity(): Identity {
  const id = useContext(IdentityContext);
  if (!id) throw new Error("useIdentity must be used inside <MarketProvider>");
  return id;
}

export function MarketProvider({
  children,
  fallback,
}: {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}) {
  const [identity, setIdentity] = useState<Identity | null>(null);

  useEffect(() => {
    let alive = true;
    void ensureIdentity().then((id) => {
      if (alive) setIdentity(id);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!identity) {
    return (
      <>
        {fallback ?? (
          <span className="text-xs text-muted-foreground">connecting…</span>
        )}
      </>
    );
  }
  return (
    <IdentityContext.Provider value={identity}>
      {children}
    </IdentityContext.Provider>
  );
}
