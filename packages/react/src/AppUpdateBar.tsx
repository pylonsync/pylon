"use client";

import type { ReactNode } from "react";
import type { AppUpdater } from "./appUpdate";
import { useAppUpdate } from "./useAppUpdate";

export interface AppUpdateBarProps {
  className?: string;
  /** Text when a new build is ready. */
  message?: ReactNode;
  /** The button label. */
  actionLabel?: ReactNode;
  /** The button label while pending writes are sent before the reload. */
  savingLabel?: ReactNode;
  /** Text on an iOS home-screen app whose name or icon changed. */
  reinstallMessage?: ReactNode;
  updater?: AppUpdater;
}

/**
 * A minimal, unstyled notice for `useAppUpdate`. Renders nothing until a
 * new build is ready or the home-screen app must be added again. Style it
 * with `className` and the `data-pylon-app-update` attribute, or build your
 * own notice with the hook.
 */
export function AppUpdateBar({
  className,
  message = "A new version of the app is available.",
  actionLabel = "Update",
  savingLabel = "Saving changes",
  reinstallMessage = "The app name or icon changed. Remove the app from the home screen and add it again.",
  updater,
}: AppUpdateBarProps) {
  const { updateReady, reinstallRecommended, applying, waitingForSync, apply } =
    useAppUpdate(updater);
  if (updateReady) {
    return (
      <div role="status" className={className} data-pylon-app-update="ready">
        <span>{message}</span>{" "}
        <button type="button" onClick={() => void apply()} disabled={applying}>
          {waitingForSync ? savingLabel : actionLabel}
        </button>
      </div>
    );
  }
  if (reinstallRecommended) {
    return (
      <div role="status" className={className} data-pylon-app-update="reinstall">
        {reinstallMessage}
      </div>
    );
  }
  return null;
}
