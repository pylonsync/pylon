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

import type { ClientChange } from "./types";

export interface PendingMutation {
  id: string;
  change: ClientChange;
  status: "pending" | "applied" | "failed";
  error?: string;
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

export class MutationQueue {
  private queue: PendingMutation[] = [];
  private persistence?: MutationQueuePersistence;

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
   *  idempotency. */
  add(change: ClientChange): string {
    const id = `mut_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    const changeWithOp: ClientChange = { ...change, op_id: id };
    this.queue.push({ id, change: changeWithOp, status: "pending" });
    this.flush();
    return id;
  }

  pending(): PendingMutation[] {
    return this.queue.filter((m) => m.status === "pending");
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

  markApplied(id: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) m.status = "applied";
    this.flush();
  }

  markFailed(id: string, error: string): void {
    const m = this.queue.find((m) => m.id === id);
    if (m) {
      m.status = "failed";
      m.error = error;
    }
    this.flush();
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

  /** Fire-and-forget persistence write. */
  private flush(): void {
    if (!this.persistence) return;
    const snapshot = this.queue.slice();
    this.persistence.saveAll(snapshot).catch((err) => {
      console.warn("[sync] mutation-queue persist failed:", err);
    });
  }
}
