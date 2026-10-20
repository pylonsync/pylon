"use client";

import {
	localUploadId,
	OUTBOX_CHANNEL,
	type FileVisibility,
	type QueuedUploadHandle,
	type SyncEngine,
	type UploadAttachTarget,
} from "@pylonsync/sync";
import { useSyncExternalStore } from "react";
import { db } from "./db";

export type { QueuedUploadHandle, UploadAttachTarget };

export interface QueueUploadOptions {
	/** The record the file belongs to, and the field that gets its id
	 *  (and `urlField`, its URL). */
	attach: UploadAttachTarget;
	filename?: string;
	contentType?: string;
	visibility?: FileVisibility;
	/** Defaults to the engine `init()` set up. */
	engine?: SyncEngine;
}

/**
 * Keep a file on the device and upload it when there is a network, then
 * set it on its record. Returns at once, also with no network.
 *
 * At once the record's `attach.field` (and `attach.urlField`) holds
 * `local:<localId>`; read it with `useFileUrl` to show the photo from the
 * device. When the upload finishes, the field gets the file id and
 * `urlField` the file URL. Both changes are ordinary queued writes.
 *
 * ```ts
 * const photoId = await db.insert("Photo", { leadId, fileId: null, url: null });
 * await queueUpload(file, {
 *   attach: { entity: "Photo", id: photoId, field: "fileId", urlField: "url" },
 * });
 * ```
 *
 * Pending and failed uploads are in `useOutbox()` with kind `"upload"`.
 * Throws `UploadQueueFullError` when the queue already holds
 * `maxQueuedUploadBytes` (200 MB by default).
 */
export function queueUpload(
	file: Blob,
	options: QueueUploadOptions,
): Promise<QueuedUploadHandle> {
	const { engine, ...rest } = options;
	return (engine ?? db.sync).queueUpload(file, rest);
}

/**
 * The URL to show for a file field: the value itself, or, for a
 * `local:<localId>` value from `queueUpload`, an object URL of the copy on
 * this device. Null for an empty value, and for a `local:` value this
 * device does not hold (another device queued it, or it was dismissed).
 *
 * ```tsx
 * const src = useFileUrl(photo.url);
 * return src ? <img src={src} /> : <Placeholder />;
 * ```
 */
export function useFileUrl(
	value: string | null | undefined,
	engine?: SyncEngine,
): string | null {
	const sync = engine ?? db.sync;
	const localId = localUploadId(value);
	return useSyncExternalStore(
		(cb) => sync.store.subscribe(cb, OUTBOX_CHANNEL),
		() => (localId ? sync.uploadLocalUrl(localId) : (value ?? null)),
		() => (localId ? null : (value ?? null)),
	);
}
