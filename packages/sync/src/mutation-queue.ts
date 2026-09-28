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
  /** Pre-mutation snapshot of the affected row, captured at optimistic-
   *  apply time for `update`/`delete`. On a server rejection,
   *  `failPushedMutation` restores this so the local replica reverts to
   *  the value the server actually holds. `null` = the row didn't exist
   *  before the mutation; `undefined` = not captured (inserts). */
  prevRow?: Row | null;
}

/**
 * Optional persistence backend. The default IndexedDB persistence
 * layer provides `savePending`/`loadPending`. Callers can supply a
 * custom backend for tests or alternative storage.
 */
export interface MutationQueuePersistence {
  saveAll(mutations: PendingMutation[]): Promise<void>;
  loadAll(): Promise<PendingMutation[]>;
}

interface OutcomeWaiter {
  resolve: () => void;
  reject: (err: MutationRejectedError) => void;
}

export class MutationQueue {
  private queue: PendingMutation[] = [];
  private persistence?: MutationQueuePersistence;
  /** Callers waiting for a mutation's outcome, by op id. Memory-only:
   *  a promise cannot outlive the page that created it. */
  private waiters = new Map<string, OutcomeWaiter[]>();

  constructor(persistence?: MutationQueuePersistence) {
    this.persistence = persistence;
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
      const existingIds = new Set(this.queue.map((m) => m.id));
      let mergedAny = false;
      for (const m of loaded) {
        if (!existingIds.has(m.id)) {
          this.queue.push(m);
          mergedAny = true;
        }
      }
      if (mergedAny) this.flush();
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
  add(change: ClientChange, prevRow?: Row | null): string {
    const id =
      typeof change.op_id === "string" && change.op_id.length > 0
        ? change.op_id
        : `mut_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    if (this.queue.some((m) => m.id === id)) return id;
    const changeWithOp: ClientChange = { ...change, op_id: id };
    this.queue.push({ id, change: changeWithOp, status: "pending", prevRow });
    this.flush();
    return id;
  }

  pending(): PendingMutation[] {
    return this.queue.filter((m) => m.status === "pending");
  }

  /** Look up a queued mutation by op id (any status). Used by the
   *  follower's mutations-failed handler to reach the captured
   *  `prevRow` for rollback. */
  get(id: string): PendingMutation | undefined {
    return this.queue.find((m) => m.id === id);
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
    for (const m of this.queue) {
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
    const m = this.queue.find((m) => m.id === id);
    if (m) m.status = "applied";
    this.flush();
    this.resolveWaiters(id);
  }

  markFailed(id: string, error: string, errorCode?: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) {
      m.status = "failed";
      m.error = error;
      if (errorCode) m.errorCode = errorCode;
    }
    this.flush();
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
    this.queue = this.queue.filter(
      (m) => m.status === "pending" || m.status === "failed",
    );
    this.flush();
  }

  /** Remove a specific mutation by id. Used by the UI after user
   *  ack of failures. */
  remove(id: string): void {
    this.queue = this.queue.filter((m) => m.id !== id);
    this.flush();
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
    this.queue = [];
    this.flush();
    // Nobody will push these writes any more. Reject their waiters so a
    // caller awaiting the outcome does not hang.
    for (const m of dropped) {
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
  private flush(): void {
    if (!this.persistence) return;
    const snapshot = this.queue.slice();
    this.persistence.saveAll(snapshot).catch((err) => {
      console.warn("[sync] mutation-queue persist failed:", err);
    });
  }
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
