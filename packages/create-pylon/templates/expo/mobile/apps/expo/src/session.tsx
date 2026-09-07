import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import {
  db,
  getReactStorage,
  guestSession,
  signOut as pylonSignOut,
  storageKey,
  useSession,
} from "@pylonsync/react-native";
import { ensurePylon } from "./pylon";
import { getOnboardingDone, setOnboardingDone as persistOnboardingDone } from "./flags";
import { configure as configurePurchases, syncEntitlements } from "./purchases";

/**
 * What the router needs to know:
 *
 *   booting     the engine and the on-device flags are still loading
 *   onboarding  first launch: show the welcome slides
 *   ready       a session exists (guest or account) and the tabs can render
 *   signedOut   no session; offer sign-in or a fresh guest
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

async function notBooted(): Promise<void> {
  throw new Error("Pylon is still booting");
}

const BOOTING: SessionValue = {
  state: "booting",
  userId: null,
  isGuest: false,
  completeOnboarding: notBooted,
  continueAsGuest: notBooted,
  signOut: notBooted,
};

/**
 * Boots Pylon, then mounts the session reader. The order matters: before
 * `init()` has run, `db.sync` would create a default engine with the
 * wrong base URL and no AsyncStorage, and the reader would stay bound to
 * it. Children render during boot, so the native splash stays up (see
 * app/_layout.tsx) instead of a blank frame.
 */
export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [boot, setBoot] = useState<{ onboardingDone: boolean } | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      await ensurePylon();
      const onboardingDone = await getOnboardingDone();
      if (alive) setBoot({ onboardingDone });
    })();
    return () => {
      alive = false;
    };
  }, []);

  if (!boot) return <Ctx.Provider value={BOOTING}>{children}</Ctx.Provider>;
  return <BootedSession initialOnboardingDone={boot.onboardingDone}>{children}</BootedSession>;
}

/** A token in AsyncStorage: the last session (guest or account) to restore. */
function hasStoredToken(): boolean {
  return Boolean(getReactStorage().get(storageKey("token")));
}

function BootedSession({
  initialOnboardingDone,
  children,
}: {
  initialOnboardingDone: boolean;
  children: React.ReactNode;
}) {
  const [onboardingDone, setOnboardingDoneState] = useState(initialOnboardingDone);
  const session = useSession(db.sync);
  const userId = session.userId;

  // Identify the purchase SDK with the Pylon user id so RevenueCat's
  // app_user_id is our id and webhook events map to our rows. After a
  // guest signs in, RevenueCat aliases the ids; re-reading the
  // entitlements moves the guest's purchase onto the account.
  useEffect(() => {
    if (!userId) return;
    void configurePurchases(userId).then((ok) => (ok ? syncEntitlements() : undefined));
  }, [userId]);

  const continueAsGuest = useCallback(async () => {
    await guestSession();
    await db.sync.notifySessionChanged();
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
    // Wait for /api/auth/me so `state` is signedOut before the caller
    // navigates, not one render later.
    await db.sync.notifySessionChanged();
  }, []);

  // Until /api/auth/me answers, `session.userId` is null because the
  // answer has not arrived, not because the user is signed out. A stored
  // token means the previous session is still the one to show: the
  // replica renders from AsyncStorage and the server confirms or rejects
  // the token on the first pull. An expired token resolves to no user and
  // the state moves to signedOut then.
  const hasSession = session.resolved ? userId != null : hasStoredToken();
  const state: SessionState = !onboardingDone ? "onboarding" : hasSession ? "ready" : "signedOut";

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
