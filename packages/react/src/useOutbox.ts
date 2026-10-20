"use client";

import {
	OUTBOX_CHANNEL,
	type Outbox,
	type OutboxItem,
	type Row,
	type SyncEngine,
} from "@pylonsync/sync";
import { useCallback, useSyncExternalStore } from "react";
import { db } from "./db";

export type { Outbox, OutboxItem };

export interface UseOutboxReturn {
	/** Writes the server has not applied yet: made offline, or waiting
	 *  for a sign-in. Oldest first. */
	pending: OutboxItem[];
	/** Writes the server rejected, with `error` and `errorCode`. Oldest
	 *  first. Each stays until it is retried or dismissed. */
	failed: OutboxItem[];
	/** Send a failed write again, with `data` merged over its fields.
	 *  Resolves once the server applies it or it is queued offline;
	 *  rejects with a `MutationRejectedError` when the server rejects it
	 *  again or another user is signed in. */
	retry: (id: string, data?: Row) => Promise<string>;
	/** Drop a failed write. */
	dismiss: (id: string) => boolean;
}

const EMPTY: Outbox = { pending: [], failed: [] };

/**
 * The writes on this device that are not on the server: pending writes
 * (offline, or waiting for a sign-in) and failed writes (the server
 * rejected them). Re-renders when a write is queued, applied, rejected,
 * retried or dismissed.
 *
 * ```tsx
 * function Outbox() {
 *   const { pending, failed, retry, dismiss } = useOutbox();
 *   return (
 *     <>
 *       {pending.length > 0 && <p>{pending.length} not synced</p>}
 *       {failed.map((w) => (
 *         <div key={w.id}>
 *           {w.entity}: {w.error}
 *           <button onClick={() => retry(w.id)}>Retry</button>
 *           <button onClick={() => dismiss(w.id)}>Dismiss</button>
 *         </div>
 *       ))}
 *     </>
 *   );
 * }
 * ```
 *
 * Pass an explicit `engine` if your app uses a non-default sync
 * instance; otherwise the hook reads the singleton wired up by `init()`.
 */
export function useOutbox(engine?: SyncEngine): UseOutboxReturn {
	const sync = engine ?? db.sync;
	const outbox = useSyncExternalStore(
		(cb) => sync.store.subscribe(cb, OUTBOX_CHANNEL),
		() => sync.outbox(),
		// The server has no queue.
		() => EMPTY,
	);
	const retry = useCallback(
		(id: string, data?: Row) => sync.retryWrite(id, data),
		[sync],
	);
	const dismiss = useCallback((id: string) => sync.dismissWrite(id), [sync]);
	return { pending: outbox.pending, failed: outbox.failed, retry, dismiss };
}
