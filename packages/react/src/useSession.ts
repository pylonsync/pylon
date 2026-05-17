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
   * Switch the active tenant. POSTs to /api/auth/select-org, verifies
   * membership server-side, resets the local replica so stale rows
   * from the previous tenant disappear, and refreshes the cached
   * session — all in one await. Throws (with `.code`/`.status`) on
   * NOT_A_MEMBER and other server errors.
   */
  selectOrg: (orgId: string) => Promise<void>;
  /** Drop the active tenant (back to no-org state). */
  clearOrg: () => Promise<void>;
  /** Revoke the current session server-side and refresh. */
  signOut: () => Promise<void>;
  /**
   * Escape hatch — refresh the cached session without mutating it.
   * Most apps shouldn't need this; the helpers above already include
   * a refresh. Useful when app code calls /api/auth/* via its own
   * client and needs the engine to observe the change.
   */
  refresh: () => Promise<void>;
}

/**
 * Subscribe to the server-resolved session held by the sync engine.
 *
 * The engine fetches `/api/auth/me` on start and on token flips, caches
 * the result, and notifies the store on change — so this hook is
 * purely a reader. The exposed helpers (selectOrg, clearOrg, signOut)
 * combine the server fetch + cache refresh + replica reset into one
 * call so app code doesn't need to remember the manual
 * `notifySessionChanged()` step after every auth mutation.
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
    selectOrg: (orgId: string) => sync.selectOrg(orgId),
    clearOrg: () => sync.clearOrg(),
    signOut: () => sync.signOut(),
    refresh: () => sync.notifySessionChanged(),
  };
}
