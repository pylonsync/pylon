"use client";

import {
	SYNC_PROGRESS_CHANNEL,
	type SyncEngine,
	type SyncProgress,
} from "@pylonsync/sync";
import { useSyncExternalStore } from "react";
import { db } from "./db";

export type { SyncProgress };

const SERVER: SyncProgress = {
	pulling: false,
	snapshot: false,
	rows: 0,
	pages: 0,
	synced: false,
};

/**
 * How far the sync engine's current pull has got: `pulling`, `snapshot`
 * (a first sync or a far-behind catch-up), `rows` and `pages` received,
 * and `synced` (a pull completed since the last replica reset).
 *
 * The session resolves as soon as `/api/auth/me` answers, before the
 * replica has filled. Render the signed-in screen from `useSession()` and
 * use this for a progress line while the data arrives. `useSyncStatus()`
 * reports the connection instead.
 *
 * ```tsx
 * function SyncLine() {
 *   const { pulling, snapshot, rows } = useSyncProgress();
 *   if (!pulling || !snapshot) return null;
 *   return <p>Loading data: {rows} records</p>;
 * }
 * ```
 */
export function useSyncProgress(engine?: SyncEngine): SyncProgress {
	const sync = engine ?? db.sync;
	return useSyncExternalStore(
		(cb) => sync.store.subscribe(cb, SYNC_PROGRESS_CHANNEL),
		() => sync.syncProgress(),
		() => SERVER,
	);
}
