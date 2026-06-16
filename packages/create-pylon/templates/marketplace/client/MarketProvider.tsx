"use client";

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import { db } from "@pylonsync/react";
import {
  cacheDisplayName,
  ensureDemoSeed,
  ensureReadSession,
  readIdentity,
  signIn as doSignIn,
  signOut as doSignOut,
  signUp as doSignUp,
  type Identity,
} from "./market";
import { LoginCard } from "./LoginCard";

// The sync engine (live queries, WebSocket) is browser-only, so every
// interactive island mounts behind this provider. It boots the client, keeps
// a guest session for READ connectivity (so the public ticker/grid run live
// for signed-out visitors), seeds the demo account, and exposes the current
// identity + auth actions. `identity` is null until the visitor signs in.

interface AuthContextValue {
  /** The signed-in user, or null when browsing anonymously. */
  identity: Identity | null;
  /** True until the client has booted + read connectivity is established. */
  ready: boolean;
  signIn: (email: string, password: string) => Promise<void>;
  signUp: (email: string, password: string, name: string) => Promise<void>;
  signOut: () => Promise<void>;
}

const Ctx = createContext<AuthContextValue | null>(null);

/** The signed-in user, or null when anonymous. */
export function useIdentity(): Identity | null {
  return useAuth().identity;
}

export function useAuth(): AuthContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAuth must be used inside <MarketProvider>");
  return v;
}

export function MarketProvider({
  children,
  fallback,
}: {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}) {
  const [ready, setReady] = useState(false);
  const [identity, setIdentity] = useState<Identity | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      await ensureReadSession();
      // Fire-and-forget: makes the demo login work + seeds the catalog.
      void ensureDemoSeed();
      if (!alive) return;
      setIdentity(readIdentity());
      setReady(true);
    })();

    const onChange = () => setIdentity(readIdentity());
    window.addEventListener("pylon-auth-changed", onChange);
    window.addEventListener("storage", onChange);
    return () => {
      alive = false;
      window.removeEventListener("pylon-auth-changed", onChange);
      window.removeEventListener("storage", onChange);
    };
  }, []);

  const signIn = useCallback(async (email: string, password: string) => {
    await doSignIn(email, password);
    setIdentity(readIdentity());
  }, []);
  const signUp = useCallback(
    async (email: string, password: string, name: string) => {
      await doSignUp(email, password, name);
      setIdentity(readIdentity());
    },
    [],
  );
  const signOut = useCallback(async () => {
    await doSignOut();
    setIdentity(readIdentity());
  }, []);

  if (!ready) {
    return (
      <>
        {fallback ?? (
          <span className="text-xs text-muted-foreground">connecting…</span>
        )}
      </>
    );
  }

  return (
    <Ctx.Provider value={{ identity, ready, signIn, signUp, signOut }}>
      {identity ? <DisplayNameSync userId={identity.userId} /> : null}
      {children}
    </Ctx.Provider>
  );
}

// Keeps the cached displayName fresh from the live User row, so identity.name
// upgrades from the email handle to the real name once the row syncs in.
function DisplayNameSync({ userId }: { userId: string }) {
  const { data } = db.useQueryOne<{ id: string; displayName?: string }>(
    "User",
    userId,
  );
  const name = data?.displayName;
  useEffect(() => {
    if (name) cacheDisplayName(name);
  }, [name]);
  return null;
}

/**
 * Gate a write surface behind sign-in. Renders the children when the visitor
 * is signed in; otherwise a prefilled demo login card. Reads (browse, ticker,
 * listing detail) don't use this — they stay public.
 */
export function AuthGate({
  children,
  title,
  blurb,
}: {
  children: React.ReactNode;
  title?: string;
  blurb?: string;
}) {
  const { identity } = useAuth();
  if (!identity) return <LoginCard title={title} blurb={blurb} />;
  return <>{children}</>;
}
