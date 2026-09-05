import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { db, guestSession, signOut as pylonSignOut, useSession } from "@pylonsync/react-native";
import { ensurePylon } from "./pylon";
import { getOnboardingDone, setOnboardingDone as persistOnboardingDone } from "./flags";
import { configure as configurePurchases } from "./purchases";

/**
 * What the router needs to know:
 *
 *   booting     the engine and the on-device flags are still loading
 *   onboarding  first launch: show the welcome slides
 *   ready       a session exists (guest or account) and the tabs can render
 *   signedOut   the user signed out explicitly; offer sign-in or a fresh guest
 */
export type SessionState = "booting" | "onboarding" | "ready" | "signedOut";

interface SessionValue {
  state: SessionState;
  userId: string | null;
  /** True while the session is an anonymous guest (no account yet). */
  isGuest: boolean;
  completeOnboarding: () => Promise<void>;
  continueAsGuest: () => Promise<void>;
  signOut: () => Promise<void>;
}

const Ctx = createContext<SessionValue | null>(null);

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [booted, setBooted] = useState(false);
  const [onboardingDone, setOnboardingDoneState] = useState(false);
  const [signedOut, setSignedOut] = useState(false);
  const session = useSession(db.sync);
  const userId = session.userId;

  useEffect(() => {
    let alive = true;
    void (async () => {
      await ensurePylon();
      const done = await getOnboardingDone();
      if (!alive) return;
      setOnboardingDoneState(done);
      setBooted(true);
    })();
    return () => {
      alive = false;
    };
  }, []);

  // Identify the purchase SDK with the Pylon user id so RevenueCat's
  // app_user_id is our id and webhook events map to our rows.
  useEffect(() => {
    if (userId) void configurePurchases(userId);
  }, [userId]);

  const continueAsGuest = useCallback(async () => {
    await guestSession();
    await db.sync.notifySessionChanged();
    setSignedOut(false);
  }, []);

  const completeOnboarding = useCallback(async () => {
    await persistOnboardingDone();
    setOnboardingDoneState(true);
    if (!db.sync.resolvedSession().userId) {
      await continueAsGuest();
    }
  }, [continueAsGuest]);

  const signOut = useCallback(async () => {
    await pylonSignOut();
    setSignedOut(true);
  }, []);

  const state: SessionState = !booted
    ? "booting"
    : !onboardingDone
      ? "onboarding"
      : userId && !signedOut
        ? "ready"
        : "signedOut";

  const value = useMemo<SessionValue>(
    () => ({
      state,
      userId,
      isGuest: Boolean(userId?.startsWith("guest_")),
      completeOnboarding,
      continueAsGuest,
      signOut,
    }),
    [state, userId, completeOnboarding, continueAsGuest, signOut],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAppSession(): SessionValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAppSession must be used inside <SessionProvider>");
  return v;
}
