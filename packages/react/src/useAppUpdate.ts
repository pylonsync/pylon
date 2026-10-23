"use client";

import { useSyncExternalStore } from "react";
import {
  appUpdate,
  INERT_APP_UPDATE_STATE,
  type AppUpdater,
  type AppUpdateState,
} from "./appUpdate";

export interface UseAppUpdateReturn extends AppUpdateState {
  /** Wait until the outbox has no pending writes, then load the newest
   *  build (the page reloads). */
  apply: () => Promise<void>;
  /** Ask the server for the latest build now. The hook also checks when
   *  the page becomes visible and every five minutes. */
  check: () => Promise<void>;
}

const serverSnapshot = () => INERT_APP_UPDATE_STATE;

/**
 * Whether a newer build of the app is deployed, and a way to load it.
 * Works in apps that set `buildManifest({ offline })`; elsewhere, under
 * `pylon dev` and during SSR, `updateReady` stays false and `apply()` does
 * nothing.
 *
 * ```tsx
 * function UpdateNotice() {
 *   const { updateReady, applying, waitingForSync, apply } = useAppUpdate();
 *   if (!updateReady) return null;
 *   return (
 *     <div role="status">
 *       A new version of the app is available.
 *       <button onClick={apply} disabled={applying}>
 *         {waitingForSync ? "Saving changes" : "Update"}
 *       </button>
 *     </div>
 *   );
 * }
 * ```
 *
 * Pass an explicit `updater` (from `createAppUpdate`) to use one other than
 * the page's.
 */
export function useAppUpdate(updater: AppUpdater = appUpdate): UseAppUpdateReturn {
  const state = useSyncExternalStore(updater.subscribe, updater.getSnapshot, serverSnapshot);
  return { ...state, apply: updater.apply, check: updater.check };
}
