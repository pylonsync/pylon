// Offline upload queue.
//
// `SyncEngine.queueUpload(file, { attach })` stores the file's bytes on the
// device and returns at once. The engine's leader tab uploads queued files
// one at a time when it has a network (`uploadBytes`: init, PUT, confirm),
// then sets the file id on the record the upload belongs to with an
// ordinary queued update, so policies and owner rules apply as for any
// write.
//
// Storage: an IndexedDB database of its own (`pylon_uploads_<app>`), so the
// bytes never sit in the replica database, whose wipe on an identity change
// would take them without saying so. Without IndexedDB (tests, React
// Native) the queue lives in memory.

import type { OutboxItem } from "./mutation-queue";
import type { TransportConfig } from "./transport";
import { uploadBytes, type FileVisibility, type UploadedFile } from "./upload";

/** Where an upload is attached when it finishes: `entity`/`id` gets
 *  `field` = the file id, and `urlField` (when set) = the file URL. */
export interface UploadAttachTarget {
  entity: string;
  id: string;
  field: string;
  urlField?: string;
}

/** One queued upload, as stored. */
export interface QueuedUpload {
  /** `upl_...`. The record's field holds `local:<localId>` until the
   *  upload finishes. */
  localId: string;
  blob: Blob;
  filename: string;
  contentType: string;
  size: number;
  visibility: FileVisibility;
  /** User id the upload was made as (`null` = anonymous). */
  owner: string | null;
  /** Made before this run knew who is signed in; stamped by the first
   *  resolved session. */
  ownerPending?: boolean;
  attach: UploadAttachTarget;
  createdAt: number;
  status: "pending" | "failed";
  error?: string;
  errorCode?: string;
  /** Set once the server confirmed the file, before the attach update is
   *  queued. A restart in between attaches without uploading again. */
  uploaded?: UploadedFile;
}

/** Persistence for queued uploads. */
export interface UploadStore {
  put(record: QueuedUpload): Promise<void>;
  delete(localId: string): Promise<void>;
  list(): Promise<QueuedUpload[]>;
}

/** The prefix of a field value that names a queued upload. */
export const LOCAL_FILE_PREFIX = "local:";

/** `local:<localId>` → `<localId>`; null for any other value. */
export function localUploadId(ref: unknown): string | null {
  return typeof ref === "string" && ref.startsWith(LOCAL_FILE_PREFIX)
    ? ref.slice(LOCAL_FILE_PREFIX.length)
    : null;
}

/** Default cap on the bytes waiting in the queue: 200 MB. */
export const DEFAULT_MAX_QUEUED_UPLOAD_BYTES = 200 * 1024 * 1024;

/** Thrown by `queueUpload` when the file would take the queue past its
 *  size limit. Past the limit the browser may evict the app's storage,
 *  which would lose every queued file, so the queue refuses instead. */
export class UploadQueueFullError extends Error {
  readonly code = "UPLOAD_QUEUE_FULL";
  readonly limitBytes: number;
  readonly queuedBytes: number;
  constructor(limitBytes: number, queuedBytes: number, fileBytes: number) {
    super(
      `The upload queue holds ${formatBytes(queuedBytes)}; this file (${formatBytes(fileBytes)}) ` +
        `would pass the ${formatBytes(limitBytes)} limit. Connect to send the queued files first.`,
    );
    this.name = "UploadQueueFullError";
    this.limitBytes = limitBytes;
    this.queuedBytes = queuedBytes;
  }
}

function formatBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${n} B`;
}

export class MemoryUploadStore implements UploadStore {
  private rows = new Map<string, QueuedUpload>();
  async put(record: QueuedUpload): Promise<void> {
    this.rows.set(record.localId, { ...record });
  }
  async delete(localId: string): Promise<void> {
    this.rows.delete(localId);
  }
  async list(): Promise<QueuedUpload[]> {
    return Array.from(this.rows.values(), (r) => ({ ...r }));
  }
}

const UPLOAD_STORE = "uploads";

/** Queued uploads in IndexedDB database `pylon_uploads_<appName>`. */
export class IndexedDBUploadStore implements UploadStore {
  private db: Promise<IDBDatabase> | null = null;
  constructor(private readonly appName = "default") {}

  private open(): Promise<IDBDatabase> {
    if (!this.db) {
      this.db = new Promise((resolve, reject) => {
        const req = indexedDB.open(`pylon_uploads_${this.appName}`, 1);
        req.onupgradeneeded = () => {
          const db = req.result;
          if (!db.objectStoreNames.contains(UPLOAD_STORE)) {
            db.createObjectStore(UPLOAD_STORE, { keyPath: "localId" });
          }
        };
        req.onsuccess = () => {
          const db = req.result;
          // Another tab deleting or upgrading the database: let it.
          db.onversionchange = () => {
            db.close();
            this.db = null;
          };
          resolve(db);
        };
        req.onerror = () => reject(req.error);
      });
      this.db.catch(() => {
        this.db = null;
      });
    }
    return this.db;
  }

  private async run<T>(
    mode: IDBTransactionMode,
    fn: (store: IDBObjectStore) => IDBRequest<T> | void,
  ): Promise<T | undefined> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(UPLOAD_STORE, mode);
      const req = fn(tx.objectStore(UPLOAD_STORE));
      tx.oncomplete = () => resolve(req ? (req.result as T) : undefined);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error ?? new Error("upload store transaction aborted"));
    });
  }

  async put(record: QueuedUpload): Promise<void> {
    await this.run("readwrite", (s) => s.put(record));
  }
  async delete(localId: string): Promise<void> {
    await this.run("readwrite", (s) => s.delete(localId));
  }
  async list(): Promise<QueuedUpload[]> {
    return ((await this.run<QueuedUpload[]>("readonly", (s) => s.getAll())) ?? []).slice();
  }
  /** Close the connection (tests; a deleteDatabase waits for it). */
  async close(): Promise<void> {
    if (!this.db) return;
    const db = await this.db.catch(() => null);
    db?.close();
    this.db = null;
  }
}

/** What the queue does with an upload, by its owner and who is signed in. */
export type UploadOwnerVerdict = "send" | "hold" | "discard";

/**
 * The owner rule for uploads, the same as for queued writes: send when the
 * upload's owner is signed in (or it was a guest's and that guest signed in
 * to an account), hold while nobody or nobody known is signed in, discard
 * when another user is signed in.
 */
export function uploadOwnerVerdict(
  owner: string | null,
  current: string | null | undefined,
): UploadOwnerVerdict {
  if (current === undefined) return "hold";
  if (owner === current) return "send";
  if (current === null) return "hold";
  if (typeof owner === "string" && owner.startsWith("guest_") && !current.startsWith("guest_")) {
    return "send";
  }
  return "discard";
}

/** Whether an upload error is worth trying again later: no network, a
 *  server error, a timeout, or a rate limit. Anything else (the type or
 *  size was refused, a policy) is the server's answer. */
export function isTransientUploadError(err: unknown): boolean {
  const status = (err as { status?: unknown })?.status;
  if (typeof status !== "number") return true;
  // 401: the session lapsed; the upload waits for the next sign-in.
  return status >= 500 || status === 401 || status === 408 || status === 429;
}

/** One queued upload as the outbox shows it. */
export function uploadOutboxItem(r: QueuedUpload): OutboxItem {
  const data: Record<string, unknown> = {
    field: r.attach.field,
    filename: r.filename,
    contentType: r.contentType,
    size: r.size,
  };
  if (r.attach.urlField) data.urlField = r.attach.urlField;
  const item: OutboxItem = {
    id: r.localId,
    entity: r.attach.entity,
    rowId: r.attach.id,
    kind: "upload",
    data,
    createdAt: r.createdAt,
  };
  if (r.status === "failed") {
    if (r.error !== undefined) item.error = r.error;
    if (r.errorCode !== undefined) item.errorCode = r.errorCode;
  }
  return item;
}

export function newUploadId(): string {
  return `upl_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

/** What `queueUpload` returns. */
export interface QueuedUploadHandle {
  localId: string;
  /** `local:<localId>`: the value the record's field holds until the
   *  upload finishes. */
  ref: string;
  /** An object URL of the file, to show it at once. Null where the
   *  platform has no `URL.createObjectURL`. */
  localUrl: string | null;
  /** Resolves with the uploaded file once it is attached; rejects with an
   *  `UploadRejectedError` when the server refuses it or another user is
   *  signed in. A page that reloads loses this promise, not the upload. */
  done: Promise<UploadedFile>;
}

/** Why a queued upload was not sent. `code` is the server's error code
 *  (`FILE_TOO_LARGE`, `POLICY_DENIED`, ...), or `DISCARDED` when another
 *  user is signed in. */
export class UploadRejectedError extends Error {
  readonly code: string;
  readonly localId: string;
  constructor(message: string, code: string, localId: string) {
    super(message);
    this.name = "UploadRejectedError";
    this.code = code;
    this.localId = localId;
  }
}

/** What the upload manager needs from the sync engine. */
export interface UploadManagerHost {
  transport(): TransportConfig;
  /** The signed-in user (`null` = anonymous), or `undefined` while it is
   *  not known (no answer from the server yet this run, or a token change
   *  in flight). */
  currentOwner(): string | null | undefined;
  /** The user a new upload belongs to; `undefined` = not known yet. */
  ownerForNewUpload(): string | null | undefined;
  /** Only the leader tab uploads. */
  isLeader(): boolean;
  isRunning(): boolean;
  /** Queue an ordinary write that sets fields on a record. */
  update(entity: string, id: string, patch: Record<string, unknown>): void;
  /** The outbox changed. */
  changed(): void;
  /** Tell other tabs to reload the queue. */
  broadcast(): void;
  /** Run `fn` after `ms` unless the engine stops. */
  later(ms: number, fn: () => void): void;
}

/**
 * The offline upload queue: storage, the one-at-a-time upload loop, the
 * owner rule, the size limit, and the outbox entries.
 */
export class UploadManager {
  private store: UploadStore = new MemoryUploadStore();
  private records = new Map<string, QueuedUpload>();
  private objectUrls = new Map<string, string>();
  private waiters = new Map<
    string,
    Array<{ resolve: (f: UploadedFile) => void; reject: (e: UploadRejectedError) => void }>
  >();
  private running = false;
  private failureCount = 0;
  private retryScheduled = false;
  private _version = 0;

  constructor(
    private readonly host: UploadManagerHost,
    private readonly maxBytes: number = DEFAULT_MAX_QUEUED_UPLOAD_BYTES,
  ) {}

  /** Bumped on every change to the queue. */
  version(): number {
    return this._version;
  }

  private changed(): void {
    this._version += 1;
    this.host.changed();
  }

  /** Swap in persistent storage and load what it holds. */
  async attachStore(store: UploadStore): Promise<void> {
    this.store = store;
    await this.reload();
  }

  /** Re-read the queue from storage (another tab changed it). */
  async reload(): Promise<void> {
    let rows: QueuedUpload[];
    try {
      rows = await this.store.list();
    } catch (err) {
      console.warn("[sync] upload queue load failed:", err);
      return;
    }
    const next = new Map(rows.map((r) => [r.localId, r]));
    for (const id of this.records.keys()) {
      if (!next.has(id)) this.revoke(id);
    }
    this.records = next;
    this.changed();
  }

  /** Queued uploads, oldest first. */
  list(): QueuedUpload[] {
    return Array.from(this.records.values()).sort((a, b) => a.createdAt - b.createdAt);
  }

  get(localId: string): QueuedUpload | undefined {
    return this.records.get(localId);
  }

  /** Bytes waiting on the device. */
  queuedBytes(): number {
    let total = 0;
    for (const r of this.records.values()) total += r.size;
    return total;
  }

  async enqueue(
    blob: Blob,
    opts: {
      attach: UploadAttachTarget;
      filename?: string;
      contentType?: string;
      visibility?: FileVisibility;
    },
  ): Promise<QueuedUploadHandle> {
    const { attach } = opts;
    if (!attach || !attach.entity || !attach.id || !attach.field) {
      throw new Error("queueUpload: attach needs entity, id and field");
    }
    const queued = this.queuedBytes();
    if (queued + blob.size > this.maxBytes) {
      throw new UploadQueueFullError(this.maxBytes, queued, blob.size);
    }
    const owner = this.host.ownerForNewUpload();
    const localId = newUploadId();
    const name =
      opts.filename ??
      (typeof File !== "undefined" && blob instanceof File ? blob.name : undefined) ??
      "upload";
    const record: QueuedUpload = {
      localId,
      blob,
      filename: name,
      contentType: opts.contentType ?? (blob.type || "application/octet-stream"),
      size: blob.size,
      visibility: opts.visibility ?? "private",
      owner: owner ?? null,
      attach: { ...attach },
      createdAt: Date.now(),
      status: "pending",
    };
    if (owner === undefined) record.ownerPending = true;
    // Stored before the call returns: a page closed right after this keeps
    // the file.
    await this.store.put(record);
    this.records.set(localId, record);
    const done = new Promise<UploadedFile>((resolve, reject) => {
      const list = this.waiters.get(localId) ?? [];
      list.push({ resolve, reject });
      this.waiters.set(localId, list);
    });
    // The caller may not await it; a rejection is still reported through
    // the outbox.
    done.catch(() => {});
    const ref = LOCAL_FILE_PREFIX + localId;
    const patch: Record<string, unknown> = { [attach.field]: ref };
    if (attach.urlField) patch[attach.urlField] = ref;
    this.host.update(attach.entity, attach.id, patch);
    this.changed();
    this.host.broadcast();
    void this.process();
    return { localId, ref, localUrl: this.localUrl(localId), done };
  }

  /** An object URL for a queued file, or null. */
  localUrl(localId: string): string | null {
    const cached = this.objectUrls.get(localId);
    if (cached) return cached;
    const record = this.records.get(localId);
    if (!record || typeof URL === "undefined" || typeof URL.createObjectURL !== "function") {
      return null;
    }
    const url = URL.createObjectURL(record.blob);
    this.objectUrls.set(localId, url);
    return url;
  }

  private revoke(localId: string): void {
    const url = this.objectUrls.get(localId);
    if (!url) return;
    this.objectUrls.delete(localId);
    // A page may still show the local copy until the record's real URL
    // renders; give it time.
    this.host.later(30_000, () => URL.revokeObjectURL(url));
  }

  private settle(localId: string, result: { file?: UploadedFile; error?: UploadRejectedError }): void {
    const list = this.waiters.get(localId);
    if (!list) return;
    this.waiters.delete(localId);
    for (const w of list) {
      if (result.file) w.resolve(result.file);
      else if (result.error) w.reject(result.error);
    }
  }

  private async remove(localId: string): Promise<void> {
    this.records.delete(localId);
    this.revoke(localId);
    await this.store.delete(localId);
  }

  /** Stamp uploads made before anyone was known to be signed in. */
  async stampPendingOwner(owner: string | null): Promise<void> {
    for (const r of this.records.values()) {
      if (!r.ownerPending) continue;
      r.owner = owner;
      delete r.ownerPending;
      await this.store.put(r);
    }
  }

  /**
   * Upload every queued file the signed-in user owns, oldest first, one at
   * a time. Stops at the first error that says the network is down and
   * tries again later (and on the next reconnect). A file the server
   * refuses is marked failed and not tried again until `retry`.
   */
  async process(): Promise<void> {
    if (this.running || !this.host.isLeader() || !this.host.isRunning()) return;
    this.running = true;
    let changed = false;
    try {
      for (const record of this.list()) {
        if (!this.records.has(record.localId) || record.status !== "pending") continue;
        if (record.ownerPending) continue;
        const verdict = uploadOwnerVerdict(record.owner, this.host.currentOwner());
        if (verdict === "hold") continue;
        if (verdict === "discard") {
          await this.remove(record.localId);
          changed = true;
          const error = new UploadRejectedError(
            "The upload was discarded because the signed-in user changed.",
            "DISCARDED",
            record.localId,
          );
          console.warn(`[sync] ${error.message} (${record.filename})`);
          this.settle(record.localId, { error });
          continue;
        }
        if (!record.uploaded) {
          try {
            record.uploaded = await uploadBytes(this.host.transport(), record.blob, {
              filename: record.filename,
              contentType: record.contentType,
              visibility: record.visibility,
            });
          } catch (err) {
            if (isTransientUploadError(err)) {
              this.scheduleRetry();
              break;
            }
            const code = (err as { code?: string }).code ?? "UPLOAD_REJECTED";
            const message = err instanceof Error ? err.message : String(err);
            record.status = "failed";
            record.error = message;
            record.errorCode = code;
            await this.store.put(record);
            changed = true;
            this.settle(record.localId, {
              error: new UploadRejectedError(message, code, record.localId),
            });
            continue;
          }
          this.failureCount = 0;
          // Remember the confirmed file before attaching, so a restart
          // here attaches instead of uploading a second copy.
          await this.store.put(record);
        }
        const file = record.uploaded;
        const patch: Record<string, unknown> = { [record.attach.field]: file.id };
        if (record.attach.urlField) patch[record.attach.urlField] = file.url;
        this.host.update(record.attach.entity, record.attach.id, patch);
        await this.remove(record.localId);
        changed = true;
        this.settle(record.localId, { file });
      }
    } finally {
      this.running = false;
      if (changed) {
        this.changed();
        this.host.broadcast();
      }
    }
  }

  private scheduleRetry(): void {
    if (this.retryScheduled) return;
    this.retryScheduled = true;
    const attempt = this.failureCount;
    this.failureCount += 1;
    const delay = Math.min(30_000, 1000 * 2 ** Math.min(attempt, 5));
    this.host.later(delay, () => {
      this.retryScheduled = false;
      void this.process();
    });
  }

  /** Queue a failed upload again. Throws `UploadRejectedError` with code
   *  `OWNER_MISMATCH` / `OWNER_UNKNOWN` as `retryWrite` does. */
  async retry(localId: string): Promise<void> {
    const record = this.records.get(localId);
    if (!record || record.status !== "failed") {
      throw new Error(`retryWrite: no failed upload with id ${localId}`);
    }
    const current = this.host.currentOwner();
    if (current === undefined || record.ownerPending) {
      throw new UploadRejectedError(
        "This upload cannot be sent again: it is not known who is signed in.",
        "OWNER_UNKNOWN",
        localId,
      );
    }
    if (uploadOwnerVerdict(record.owner, current) !== "send") {
      throw new UploadRejectedError(
        "This upload was made by another user and cannot be sent as the signed-in user.",
        "OWNER_MISMATCH",
        localId,
      );
    }
    record.status = "pending";
    delete record.error;
    delete record.errorCode;
    await this.store.put(record);
    this.changed();
    this.host.broadcast();
    await this.process();
  }

  /** Drop a failed upload and its bytes. False when there is no failed
   *  upload with that id. The record keeps its `local:` value. */
  dismiss(localId: string): boolean {
    const record = this.records.get(localId);
    if (!record || record.status !== "failed") return false;
    this.remove(localId)
      .catch((err) => console.warn("[sync] upload queue delete failed:", err))
      .finally(() => this.host.broadcast());
    this.changed();
    return true;
  }
}
