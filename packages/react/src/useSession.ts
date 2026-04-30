"use client";

import { SyncEngine, type ResolvedSession } from "@pylonsync/sync";
import { useEffect, useSyncExternalStore } from "react";

export type { ResolvedSession };

export interface UseSessionReturn {
  /** Server-resolved session. `userId=null` means anonymous. */
  session: ResolvedSession;
  /** Convenience accessors. */
  userId: string | null;
  tenantId: string | null;
  isAdmin: boolean;
  isAuthenticated: boolean;
  /**
   * Force a refresh of the cached session. Call after sign-in, sign-out,
   * or `/api/auth/select-org` so the UI and sync engine pick up the
   * change before the next pull/reconnect.
   */
  refresh: () => Promise<void>;
}

/**
 * Subscribe to the server-resolved session held by the sync engine.
 *
 * The engine fetches `/api/auth/me` on start and on token flips, caches
 * the result, and notifies the store on change — so this hook is
 * purely a reader. Mutations (login/logout/select-org) are still the
 * caller's responsibility; after them, invoke `refresh()` (or
 * `engine.notifySessionChanged()`) to pull the new session immediately.
 */
export function useSession(sync: SyncEngine): UseSessionReturn {
  const session = useSyncExternalStore(
    (cb) => sync.store.subscribe(cb),
    () => sync.resolvedSession(),
    () => sync.resolvedSession(),
  );

  // Watch the localStorage token key. If another tab signs in/out, or the
  // app writes a new token without going through `notifySessionChanged`,
  // our cached session still matches the old identity until the next
  // pull. A `storage` event covers the cross-tab case; a mount-time
  // refresh covers the same-tab write-then-mount race.
  useEffect(() => {
    void sync.notifySessionChanged();
    const onStorage = (e: StorageEvent) => {
      // Only refresh when the key that actually changed looks like a
      // pylon token. Skip unrelated keys so we don't generate an
      // /api/auth/me flood from noisy apps.
      if (!e.key) return;
      if (e.key.startsWith("pylon_") || e.key.startsWith("pylon:")) {
        if (e.key.endsWith("token") || e.key.endsWith(":token")) {
          void sync.notifySessionChanged();
        }
      }
    };
    if (typeof window !== "undefined") {
      window.addEventListener("storage", onStorage);
      return () => window.removeEventListener("storage", onStorage);
    }
    return undefined;
  }, [sync]);

  return {
    session,
    userId: session.userId,
    tenantId: session.tenantId,
    isAdmin: session.isAdmin,
    isAuthenticated: session.userId != null,
    refresh: () => sync.notifySessionChanged(),
  };
}
