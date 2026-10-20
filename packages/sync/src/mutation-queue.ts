// ---------------------------------------------------------------------------
// Offline-safe write queue.
//
// Memory-only by default. When a persistence backend is attached
// (`attachPersistence`), every state transition writes through to
// disk and `hydrate()` restores pending/failed mutations on startup.
//
// The mutation id is reused as the server-side `op_id` for idempotent
// replay — a retry carrying the same id short-circuits on the server
// instead of re-applying.
// ---------------------------------------------------------------------------

import type { ClientChange, Row } from "./types";

/**
 * Thrown by `SyncEngine.insert` / `update` / `delete` when the server
 * rejects the write (policy denial, validation, conflict). The optimistic
 * change is rolled back before the promise rejects.
 *
 * `code` is the server's error code when it sent one (`POLICY_DENIED`,
 * `VALIDATION_FAILED`, ...). `DISCARDED` means the write never reached the
 * server: an identity change dropped it from the queue.
 */
export class MutationRejectedError extends Error {
  readonly code: string;
  readonly opId: string;
  readonly entity: string;
  readonly rowId: string;
  readonly kind: "insert" | "update" | "delete";

  constructor(
    message: string,
    opts: {
      code?: string;
      opId: string;
      entity: string;
      rowId: string;
      kind: "insert" | "update" | "delete";
    },
  ) {
    super(message);
    this.name = "MutationRejectedError";
    this.code = opts.code ?? "REJECTED";
    this.opId = opts.opId;
    this.entity = opts.entity;
    this.rowId = opts.rowId;
    this.kind = opts.kind;
  }
}

export interface PendingMutation {
  id: string;
  change: ClientChange;
  status: "pending" | "applied" | "failed";
  error?: string;
  /** Server error code for a failed mutation, when the server sent one. */
  errorCode?: string;
  /** When the write was made, in ms since the epoch. Missing on writes
   *  queued by an older client; those sort first. */
  createdAt?: number;
  /** User id the write was made as (`null` = anonymous). Persisted with
   *  the queue so a write is only ever pushed as the user who made it.
   *  `undefined` = unknown (queued by an older client). */
  owner?: string | null;
  /** The write was made before this run knew who is signed in (no
   *  resolved session and no saved identity). It is held until the first
   *  session resolves, which stamps `owner`. */
  ownerPending?: boolean;
  /** Pre-mutation snapshot of the affected row, captured at optimistic-
   *  apply time for `update`/`delete`. On a server rejection,
   *  `failPushedMutation` restores this so the local replica reverts to
   *  the value the server actually holds. `null` = the row didn't exist
   *  before the mutation; `undefined` = not captured (inserts). */
  prevRow?: Row | null;
}

/** One write as the outbox shows it. */
export interface OutboxItem {
  /** Op id. Pass it to `retryWrite` / `dismissWrite`. */
  id: string;
  entity: string;
  rowId: string;
  /** `"upload"` is a file from `queueUpload`; `entity`/`rowId` is the
   *  record it attaches to. */
  kind: "insert" | "update" | "delete" | "upload";
  /** The fields the write sets. Empty for a delete. For an upload:
   *  `field`, `urlField`, `filename`, `contentType`, `size`. */
  data: Row;
  /** When the write was made, in ms since the epoch (0 when unknown). */
  createdAt: number;
  /** The server's message, for a failed write. */
  error?: string;
  /** The server's error code (`POLICY_DENIED`, `VALIDATION_FAILED`, ...),
   *  for a failed write. */
  errorCode?: string;
}

/** Writes not yet on the server, and writes the server rejected. */
export interface Outbox {
  pending: OutboxItem[];
  failed: OutboxItem[];
}

export function toOutboxItem(m: PendingMutation): OutboxItem {
  const item: OutboxItem = {
    id: m.id,
    entity: m.change.entity,
    rowId: m.change.row_id,
    kind: m.change.kind,
    data: { ...(m.change.data ?? {}) },
    createdAt: m.createdAt ?? 0,
  };
  if (m.status === "failed") {
    if (m.error !== undefined) item.error = m.error;
    if (m.errorCode !== undefined) item.errorCode = m.errorCode;
  }
  return item;
}

/**
 * Optional persistence backend. The default IndexedDB persistence
 * layer supports incremental writes with `saveChanges`. Custom backends
 * can keep the full-snapshot `saveAll` contract.
 */
export interface MutationQueuePersistence {
  saveAll(mutations: PendingMutation[]): Promise<void>;
  loadAll(): Promise<PendingMutation[]>;
  /** Optional ordered, atomic delta write. Unchanged entries stay on disk. */
  saveChanges?(upserts: PendingMutation[], removedIds: string[]): Promise<void>;
}

interface OutcomeWaiter {
  resolve: () => void;
  reject: (err: MutationRejectedError) => void;
}

export class MutationQueue {
  private queue = new Map<string, PendingMutation>();
  private persistence?: MutationQueuePersistence;
  /** Callers waiting for a mutation's outcome, by op id. Memory-only:
   *  a promise cannot outlive the page that created it. */
  private waiters = new Map<string, OutcomeWaiter[]>();
  /** Called after every change to what the queue holds or to a write's
   *  status. The engine turns it into a store notification so the outbox
   *  re-renders. */
  private changeListener: (() => void) | null = null;
  /** Bumped on every change; lets readers cache a snapshot. */
  private _version = 0;

  constructor(persistence?: MutationQueuePersistence) {
    this.persistence = persistence;
  }

  /** Set the one listener called after each queue change. */
  setChangeListener(listener: (() => void) | null): void {
    this.changeListener = listener;
  }

  /** Changes so far. Equal versions mean the same contents. */
  version(): number {
    return this._version;
  }

  private changed(): void {
    this._version += 1;
    this.changeListener?.();
  }

  /**
   * Attach a persistence backend after construction. The SyncEngine
   * swaps in IndexedDB-backed persistence once the DB has opened.
   * Public so it doesn't need a `// @ts-expect-error` to reach in
   * from the same package.
   */
  attachPersistence(persistence: MutationQueuePersistence): void {
    this.persistence = persistence;
  }

  /** Load persisted queue state. Call once at startup. */
  async hydrate(): Promise<void> {
    if (!this.persistence) return;
    try {
      const loaded = await this.persistence.loadAll();
      // Merge in-memory with on-disk. An `add()` that ran while
      // hydrate was awaiting `loadAll()` will already have flushed a
      // snapshot that didn't include the loaded rows — re-flush
      // after merge so disk matches memory again.
      let mergedAny = false;
      for (const m of loaded) {
        if (!this.queue.has(m.id)) {
          this.queue.set(m.id, m);
          mergedAny = true;
        }
      }
      if (mergedAny) {
        this.flush();
        this.changed();
      }
    } catch (err) {
      // Broken storage shouldn't prevent the app from running — warn
      // and degrade to memory-only mode.
      console.warn("[sync] mutation-queue hydrate failed:", err);
    }
  }

  /** Add a pending mutation. Returns the op_id used for server
   *  idempotency. If the change ALREADY carries an op_id (e.g., it
   *  was forwarded from another tab via multi-tab broadcast), reuse
   *  that id so the leader and follower agree on the identifier the
   *  server will dedupe against. Likewise we skip the add when an
   *  entry with the same op_id is already queued — a follower
   *  retrying its forward of the same op shouldn't double-queue on
   *  the leader. */
  add(
    change: ClientChange,
    prevRow?: Row | null,
    owner?: string | null,
    ownerPending?: boolean,
    createdAt?: number,
  ): string {
    const id =
      typeof change.op_id === "string" && change.op_id.length > 0
        ? change.op_id
        : newOpId();
    if (this.queue.has(id)) return id;
    const changeWithOp: ClientChange = { ...change, op_id: id };
    const entry: PendingMutation = {
      id,
      change: changeWithOp,
      status: "pending",
      prevRow,
      createdAt: createdAt ?? Date.now(),
    };
    if (owner !== undefined) entry.owner = owner;
    if (ownerPending) entry.ownerPending = true;
    this.queue.set(id, entry);
    this.flush([entry]);
    this.changed();
    return id;
  }

  pending(): PendingMutation[] {
    const pending: PendingMutation[] = [];
    for (const mutation of this.queue.values()) {
      if (mutation.status === "pending") pending.push(mutation);
    }
    return pending;
  }

  /** Writes the server rejected, oldest first. They stay until the user
   *  retries or dismisses them. */
  failed(): PendingMutation[] {
    return byCreatedAt(Array.from(this.queue.values()).filter((m) => m.status === "failed"));
  }

  /**
   * Queue a failed write again under a new op id, with `data` (when
   * given) merged over the original change's fields. The failed entry is
   * removed in the same persistence write, so a reload never sees both.
   * The new write keeps the original's owner and `createdAt`. `prevRow`
   * is the row before the caller re-applied the write locally, for the
   * rollback if the server rejects it again. Returns the new op id.
   *
   * The server forgets a rejected op id (it never applied it), so the new
   * id cannot be applied alongside the old one.
   */
  retry(id: string, data: Row | undefined, prevRow: Row | null | undefined): string {
    const m = this.queue.get(id);
    if (!m || m.status !== "failed") {
      throw new Error(`no failed write with id ${id}`);
    }
    const newId = newOpId();
    const change: ClientChange = { ...m.change, op_id: newId };
    if (data !== undefined && m.change.kind !== "delete") {
      change.data = { ...(m.change.data ?? {}), ...data };
      if (m.change.kind === "insert") change.data.id = m.change.row_id;
    }
    const entry: PendingMutation = {
      id: newId,
      change,
      status: "pending",
      prevRow,
      createdAt: m.createdAt,
    };
    if (m.owner !== undefined) entry.owner = m.owner;
    this.queue.delete(id);
    this.queue.set(newId, entry);
    this.flush([entry], [id]);
    this.changed();
    return newId;
  }

  /** Drop a failed write after the user has seen it. Returns false when
   *  there is no failed write with that id (pending writes cannot be
   *  dismissed: they may already be on the server). */
  dismiss(id: string): boolean {
    const m = this.queue.get(id);
    if (!m || m.status !== "failed") return false;
    this.queue.delete(id);
    this.flush([], [id]);
    this.changed();
    return true;
  }

  /** Look up a queued mutation by op id (any status). Used by the
   *  follower's mutations-failed handler to reach the captured
   *  `prevRow` for rollback. */
  get(id: string): PendingMutation | undefined {
    return this.queue.get(id);
  }

  /**
   * Set of `${entity}/${row_id}` keys for every mutation currently
   * in Pending or Failed state. Used by reconcile() to skip rows
   * whose canonical state on the server hasn't caught up with the
   * local optimistic ghost yet — otherwise reconcile would tombstone
   * the row (it's not yet on the server) and the still-pending push
   * would later re-apply against the tombstone, fighting the local
   * replica.
   *
   * Failed mutations are included too: a user-visible failure is
   * recoverable, and sweeping the row would discard the local edit
   * the user is still trying to push.
   */
  pendingRowKeys(): Set<string> {
    const out = new Set<string>();
    for (const m of this.queue.values()) {
      if (m.status === "pending" || m.status === "failed") {
        out.add(`${m.change.entity}/${m.change.row_id}`);
      }
    }
    return out;
  }

  /**
   * Resolve once the server applies the mutation or the mutation is
   * queued behind a transient failure (`settleQueued`); reject with a
   * `MutationRejectedError` once the server rejects it. Settles at once
   * when the mutation already reached a terminal state.
   */
  waitForOutcome(id: string): Promise<void> {
    const m = this.get(id);
    if (m?.status === "applied") return Promise.resolve();
    if (m?.status === "failed") return Promise.reject(rejectionFor(m));
    return new Promise<void>((resolve, reject) => {
      const list = this.waiters.get(id) ?? [];
      list.push({ resolve, reject });
      this.waiters.set(id, list);
    });
  }

  /** Resolve the waiters of a mutation that stays queued after a
   *  transient push failure (offline, 5xx). The mutation stays pending
   *  and is retried; a later rejection shows up as `status: "failed"`. */
  settleQueued(id: string): void {
    this.resolveWaiters(id);
  }

  markApplied(id: string): void {
    this.markAppliedMany([id]);
  }

  /** Persist one response batch once, then settle each caller. */
  markAppliedMany(ids: string[]): void {
    const changed: PendingMutation[] = [];
    for (const id of new Set(ids)) {
      const m = this.queue.get(id);
      if (m) {
        m.status = "applied";
        changed.push(m);
      }
    }
    if (changed.length > 0) {
      this.flush(changed);
      this.changed();
    }
    for (const id of ids) this.resolveWaiters(id);
  }

  markFailed(id: string, error: string, errorCode?: string): void {
    const m = this.queue.get(id);
    if (m) {
      m.status = "failed";
      m.error = error;
      if (errorCode) m.errorCode = errorCode;
    }
    if (m) {
      this.flush([m]);
      this.changed();
    }
    const waiters = this.waiters.get(id);
    if (!waiters) return;
    this.waiters.delete(id);
    const err = m
      ? rejectionFor(m)
      : new MutationRejectedError(error, {
          code: errorCode,
          opId: id,
          entity: "",
          rowId: "",
          kind: "insert",
        });
    for (const w of waiters) w.reject(err);
  }

  private resolveWaiters(id: string): void {
    const waiters = this.waiters.get(id);
    if (!waiters) return;
    this.waiters.delete(id);
    for (const w of waiters) w.resolve();
  }

  /**
   * Prune applied mutations. Failed mutations are KEPT so the UI can
   * surface them to the user and so retries are possible.
   */
  clear(): void {
    const removedIds: string[] = [];
    for (const [id, m] of this.queue) {
      if (m.status === "applied") {
        this.queue.delete(id);
        removedIds.push(id);
      }
    }
    if (removedIds.length > 0) {
      this.flush([], removedIds);
      this.changed();
    }
  }

  /** Stamp every write whose owner was pending with the user the first
   *  resolved session names. */
  stampPendingOwner(owner: string | null): void {
    const changed: PendingMutation[] = [];
    for (const m of this.queue.values()) {
      if (m.ownerPending) {
        m.owner = owner;
        delete m.ownerPending;
        changed.push(m);
      }
    }
    if (changed.length > 0) this.flush(changed);
  }

  /** Drop one mutation that must never be pushed (it belongs to another
   *  identity) and reject its waiters with code `DISCARDED`. Does not
   *  touch the local store. */
  discard(id: string): void {
    const m = this.get(id);
    this.queue.delete(id);
    this.flush([], [id]);
    if (m) this.changed();
    const waiters = this.waiters.get(id);
    if (!waiters) return;
    this.waiters.delete(id);
    const err = new MutationRejectedError(
      "The write was discarded because the signed-in identity changed.",
      {
        code: "DISCARDED",
        opId: id,
        entity: m?.change.entity ?? "",
        rowId: m?.change.row_id ?? "",
        kind: m?.change.kind ?? "insert",
      },
    );
    for (const w of waiters) w.reject(err);
  }

  /** Remove a specific mutation by id. Used by the UI after user
   *  ack of failures. */
  remove(id: string): void {
    if (!this.queue.delete(id)) return;
    this.flush([], [id]);
    this.changed();
  }

  /**
   * Drop EVERY mutation — pending, failed, and applied — then flush the
   * empty queue to disk. Used by the engine's identity-flip reset
   * (`resetReplica({ wipeMutations: true })`): the queued offline writes
   * belong to the OUTGOING identity, and replaying them under the new
   * session would attribute one user's writes to another (or get
   * policy-rejected on the server). Distinct from `clear()`, which only
   * prunes already-applied entries and deliberately PRESERVES pending +
   * failed writes — the 410-RESYNC same-user path relies on that so
   * offline writes survive a snapshot refresh.
   */
  clearAll(): void {
    const dropped = this.queue;
    this.queue = new Map();
    this.flush();
    if (dropped.size > 0) this.changed();
    // Nobody will push these writes any more. Reject their waiters so a
    // caller awaiting the outcome does not hang.
    for (const m of dropped.values()) {
      const waiters = this.waiters.get(m.id);
      if (!waiters) continue;
      this.waiters.delete(m.id);
      const err = new MutationRejectedError(
        "The write was discarded because the signed-in identity changed.",
        {
          code: "DISCARDED",
          opId: m.id,
          entity: m.change.entity,
          rowId: m.change.row_id,
          kind: m.change.kind,
        },
      );
      for (const w of waiters) w.reject(err);
    }
  }

  /** Fire-and-forget persistence write. */
  private flush(upserts?: PendingMutation[], removedIds: string[] = []): void {
    if (!this.persistence) return;
    // Copy mutable status/owner fields before handing data to an async backend.
    const write = upserts && this.persistence.saveChanges
      ? this.persistence.saveChanges(upserts.map((m) => ({ ...m })), removedIds)
      : this.persistence.saveAll(Array.from(this.queue.values(), (m) => ({ ...m })));
    write.catch((err) => {
      console.warn("[sync] mutation-queue persist failed:", err);
    });
  }
}

/** A new op id. The server deduplicates pushes by it. */
function newOpId(): string {
  return `mut_${Date.now()}_${Math.random().toString(36).slice(2)}`;
}

/** Oldest first; writes without `createdAt` (older clients) first of all. */
export function byCreatedAt(list: PendingMutation[]): PendingMutation[] {
  return list.sort((a, b) => (a.createdAt ?? 0) - (b.createdAt ?? 0));
}

function rejectionFor(m: PendingMutation): MutationRejectedError {
  return new MutationRejectedError(m.error ?? "The server rejected the write.", {
    code: m.errorCode,
    opId: m.id,
    entity: m.change.entity,
    rowId: m.change.row_id,
    kind: m.change.kind,
  });
}
