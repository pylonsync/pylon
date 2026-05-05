"use client";

import {
	type SyncConnectionStatus,
	type SyncEngine,
} from "@pylonsync/sync";
import { useSyncExternalStore } from "react";
import { db } from "./db";

export type { SyncConnectionStatus };

/**
 * Subscribe to the sync engine's connection status. Re-renders whenever
 * the WebSocket transitions ("connecting" → "connected" → "reconnecting"
 * → "offline"), so apps can render a small indicator without polling
 * or wiring their own event listeners.
 *
 * Common use case: surface a "reconnecting…" banner during the cold
 * start after a Fly autostop or a flaky network. Live queries keep
 * returning the last-known data during reconnect, so without this hook
 * users see stale data with no signal that anything is wrong.
 *
 * ```tsx
 * function ConnectionBanner() {
 *   const status = useSyncStatus();
 *   if (status === "connected") return null;
 *   return (
 *     <div className="banner">
 *       {status === "reconnecting" ? "Reconnecting…" : "Offline"}
 *     </div>
 *   );
 * }
 * ```
 *
 * Pass an explicit `engine` if your app uses a non-default sync
 * instance; otherwise omit and the hook reads the singleton wired up
 * by `init()`.
 */
export function useSyncStatus(engine?: SyncEngine): SyncConnectionStatus {
	const sync = engine ?? db.sync;
	return useSyncExternalStore(
		(cb) => sync.store.subscribe(cb),
		() => sync.connectionStatus(),
		// SSR snapshot — engine is offline until start() runs in the
		// browser, so claiming "connecting" here would mislead consumers
		// rendered server-side.
		() => "offline" as SyncConnectionStatus,
	);
}
