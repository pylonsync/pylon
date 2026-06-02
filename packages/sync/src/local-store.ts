// ---------------------------------------------------------------------------
// In-memory replica of server state.
//
// Tracks tables (entity → id → row), tombstones (delete seqs that
// fence out late-arriving stale inserts), and optimistic tombstones
// (pending client deletes that block incoming inserts until the
// server's authoritative delete arrives). Persistence backends
// (IndexedDB) wire through `_persistFn`.
// ---------------------------------------------------------------------------

import type { ChangeEvent, Row } from "./types";

export class LocalStore {
  private tables: Map<string, Map<string, Row>> = new Map();
  /**
   * `(entity, row_id) → deletedAt seq`. A row in this map has been
   * deleted; any insert/update event older than its tombstone seq is
   * ignored so an out-of-order replay can't resurrect it. Without
   * this, a delete followed by a reconnect-driven replay of the
   * original insert would re-materialize the row.
   *
   * Real server-issued tombstones use the event's own seq. Optimistic
   * client tombstones live in `optimisticTombstones` instead.
   */
  private tombstones: Map<string, Map<string, number>> = new Map();
  /**
   * Pending optimistic deletes — `(entity, row_id)` pairs the client
   * has dropped but the server hasn't yet confirmed. Stored
   * separately from `tombstones` so the real server-issued delete
   * (with its real, smaller seq) can supersede the optimistic entry
   * without being max-merged out.
   *
   * Invariant: a future legitimate re-create of the same id must
   * succeed once the server's authoritative delete arrives. Test:
   * `optimistic_delete_releases_id_when_server_confirms`.
   */
  private optimisticTombstones: Map<string, Set<string>> = new Map();
  private listeners: Set<() => void> = new Set();

  /** Get all rows for an entity. */
  list(entity: string): Row[] {
    const table = this.tables.get(entity);
    if (!table) return [];
    return Array.from(table.values());
  }

  /** Get a row by ID. */
  get(entity: string, id: string): Row | null {
    return this.tables.get(entity)?.get(id) ?? null;
  }

  /** Snapshot of every entity name with at least one local row. Used
   *  by `SyncEngine.reconcile` to know which tables to diff against
   *  server truth. */
  entityNames(): string[] {
    const names: string[] = [];
    for (const [name, table] of this.tables) {
      if (table.size > 0) names.push(name);
    }
    return names;
  }

  /**
   * Remove a row whose absence was confirmed by the server-truth
   * reconciler. Records a tombstone at `tombstoneSeq` so a stale
   * insert/update replayed afterwards (e.g. a slow WS frame) can't
   * resurrect it. Callers pass the current sync cursor — any future
   * change events have higher seqs and pass the tombstone check,
   * while older replays are filtered.
   *
   * Differs from `optimisticDelete`: this is "the server says this
   * row is gone right now"; that one is "the client wants this row
   * gone before the server has confirmed."
   */
  reconcileRemove(entity: string, id: string, tombstoneSeq: number): boolean {
    const table = this.tables.get(entity);
    if (!table || !table.has(id)) return false;
    table.delete(id);
    this.recordTombstone(entity, id, tombstoneSeq);
    return true;
  }

  /**
   * Per-subscriber row revocation: the server signaled that the
   * current client lost read access to a specific row. ALWAYS
   * records the tombstone (even when the row isn't in memory), so
   * a stale insert/update event that arrives after the revocation
   * can't resurrect the row.
   *
   * `reconcileRemove` returns early when the row is absent — that's
   * the right behavior for "did anything change?" reconcile passes,
   * but wrong here: a CRDT-only consumer using `useLoroDoc` without
   * a JSON row never had the row in `tables` to begin with, and
   * without the tombstone any future server-issued insert
   * (legitimate re-grant or a slow stale frame) would land.
   *
   * Returns true if the row was present + removed; the tombstone
   * is recorded regardless.
   */
  revokeRow(entity: string, id: string, tombstoneSeq: number): boolean {
    const removed = this.tables.get(entity)?.delete(id) ?? false;
    this.recordTombstone(entity, id, tombstoneSeq);
    return removed;
  }

  private isTombstoned(entity: string, id: string, at_seq?: number): boolean {
    if (this.optimisticTombstones.get(entity)?.has(id)) return true;
    const tombSeq = this.tombstones.get(entity)?.get(id);
    if (tombSeq === undefined) return false;
    // No-seq caller means "this change has no provenance" — treat as
    // older than any tombstone (safer default than admitting it).
    if (at_seq === undefined) return true;
    return at_seq < tombSeq;
  }

  private recordTombstone(entity: string, id: string, seq: number): void {
    // A real (server-issued) tombstone supersedes any pending
    // optimistic entry for this id. Drop the optimistic guard so a
    // future legitimate re-create of the same id isn't blocked.
    this.optimisticTombstones.get(entity)?.delete(id);
    if (!this.tombstones.has(entity)) {
      this.tombstones.set(entity, new Map());
    }
    const existing = this.tombstones.get(entity)!.get(id);
    if (existing === undefined || seq > existing) {
      this.tombstones.get(entity)!.set(id, seq);
    }
  }

  /** Apply a change event to the local store. */
  applyChange(change: ChangeEvent): void {
    if (!this.tables.has(change.entity)) {
      this.tables.set(change.entity, new Map());
    }
    const table = this.tables.get(change.entity)!;

    // Drop insert/update events that arrive AFTER a delete for the
    // same row. The tombstone map records the seq of the delete;
    // anything strictly older is a stale resurrect.
    if (
      (change.kind === "insert" || change.kind === "update") &&
      this.isTombstoned(change.entity, change.row_id, change.seq)
    ) {
      return;
    }

    switch (change.kind) {
      case "insert":
        if (change.data) {
          // Spread data FIRST, then force `id = change.row_id` —
          // otherwise an `id` field in `data` could shadow the
          // authoritative row_id and corrupt the replica's primary key
          // on reload.
          table.set(change.row_id, {
            ...change.data,
            id: change.row_id,
          });
        }
        break;
      case "update":
        if (change.data) {
          const existing = table.get(change.row_id) ?? { id: change.row_id };
          table.set(change.row_id, {
            ...existing,
            ...change.data,
            id: change.row_id,
          });
        }
        break;
      case "delete":
        table.delete(change.row_id);
        this.recordTombstone(change.entity, change.row_id, change.seq);
        break;
    }
  }

  /** Apply multiple changes synchronously. Persistence runs fire-
   *  and-forget. Prefer `applyChangesAsync` when you plan to advance
   *  a cursor after — otherwise a crash can save the cursor before
   *  rows hit disk, causing permanent missed changes on restart. */
  applyChanges(changes: ChangeEvent[]): void {
    for (const change of changes) {
      this.applyChange(change);
    }
    this.notify();

    if (this._persistFn) {
      for (const change of changes) {
        const merged = this.hydrateFromMemory(change);
        void this._persistFn(merged);
      }
    }
  }

  /**
   * Apply + persist, awaiting disk writes before returning. Callers
   * that are about to advance a cursor based on `changes` MUST use
   * this path — otherwise cursor durability is broken: a crash
   * between the memory apply and the eventual disk write can persist
   * a cursor that's ahead of the replica, skipping those rows
   * forever on restart.
   *
   * Returns `true` when every persist write reached disk durably,
   * `false` when at least one degraded (quota / abort). The engine
   * uses the result to hold the PERSISTED cursor back: a row that
   * didn't reach disk must not be skipped by an advanced on-disk
   * cursor on the next cold start. The in-memory replica always
   * reflects the change regardless.
   */
  async applyChangesAsync(changes: ChangeEvent[]): Promise<boolean> {
    for (const change of changes) {
      this.applyChange(change);
    }
    this.notify();
    let allDurable = true;
    if (this._persistFn) {
      // Sequential await — concurrent IDB writes can resolve out of
      // order, racing an update behind its own delete on disk. The
      // engine's `applyQueue` sequences events into here; we
      // preserve that order down to the disk layer.
      for (const change of changes) {
        const result = this._persistFn(this.hydrateFromMemory(change));
        if (result instanceof Promise) {
          const durable = await result;
          if (durable === false) allDurable = false;
        }
      }
    }
    return allDurable;
  }

  /**
   * Reshape a change event so its `data` field matches the row as it
   * now exists in memory after `applyChange` merged the patch.
   * Persistence callers (IndexedDB) save the full row, which only
   * works if they receive the full row. Deletes pass through
   * untouched.
   */
  private hydrateFromMemory(change: ChangeEvent): ChangeEvent {
    if (change.kind === "delete") return change;
    const merged = this.tables.get(change.entity)?.get(change.row_id);
    if (!merged) return change;
    return { ...change, data: merged };
  }

  /** Persistence callback for auto-saving changes. Returns
   *  `Promise<boolean>` (true = durable, false = degraded) so
   *  `applyChangesAsync` can gate the on-disk cursor on durability.
   *  Void-returning callbacks are accepted for backwards compatibility
   *  (treated as durable / fire-and-forget). */
  _persistFn: ((change: ChangeEvent) => void | Promise<boolean>) | null = null;

  /** Subscribe to store changes. Returns unsubscribe function. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }

  /** Apply an optimistic insert. Returns a temporary id. */
  optimisticInsert(entity: string, data: Row): string {
    const tempId = `_pending_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    if (!this.tables.has(entity)) {
      this.tables.set(entity, new Map());
    }
    this.tables.get(entity)!.set(tempId, { id: tempId, ...data });
    this.notify();
    return tempId;
  }

  /**
   * Apply an optimistic insert with a caller-provided id.
   *
   * Used by `useMutation({ optimistic })`: the React hook generates a
   * Pylon-shaped id (40-char hex via `generateId()`), threads it
   * through the mutation args as `_optimisticId`, and the server
   * function honors it on `ctx.db.insert("Entity", { id, ... })`.
   * Because the optimistic ghost and the canonical row share the
   * same row_id, the WS broadcast that follows lands as a field-
   * level merge on top of the optimistic — no delete-then-replace
   * flash, no temp-row swap.
   */
  optimisticInsertWithId(entity: string, id: string, data: Row): void {
    if (!this.tables.has(entity)) {
      this.tables.set(entity, new Map());
    }
    this.tables.get(entity)!.set(id, { ...data, id });
    this.notify();
  }

  /**
   * Roll back an optimistic insert without leaving a tombstone.
   *
   * Counterpart to `optimisticInsertWithId`. On rejection the ghost
   * row must go away, but we MUST NOT leave a tombstone — a future
   * legitimate insert with the same id (user retries, workflow
   * eventually creates the row) must not be blocked.
   * `optimisticDelete` records a MAX_SAFE_INTEGER tombstone, which is
   * the wrong semantic here; this is a plain remove.
   */
  rollbackOptimisticInsert(entity: string, id: string): void {
    const removed = this.tables.get(entity)?.delete(id);
    if (removed) this.notify();
  }

  /** Apply an optimistic update. */
  optimisticUpdate(entity: string, id: string, data: Partial<Row>): void {
    const table = this.tables.get(entity);
    if (!table) return;
    const existing = table.get(id);
    if (existing) {
      table.set(id, { ...existing, ...data });
      this.notify();
    }
  }

  /**
   * Undo a rejected optimistic update/delete by restoring the row to its
   * captured pre-mutation value and clearing any optimistic tombstone
   * for it. `failPushedMutation` calls this when the server rejects an
   * update (restore the prior field values) or a delete (bring the row
   * back AND un-fence it so the row — and any future server insert of
   * the id — isn't blocked by the lingering optimistic tombstone).
   *
   * `prev === null` means the row didn't exist before the mutation
   * (e.g. an update on a row that was itself an un-acked insert) — in
   * that case we just remove it + clear the fence.
   *
   * A REAL (server-issued) tombstone wins over the restore: if an
   * authoritative delete/revocation for this id landed on the applyQueue
   * while the rejected push was in flight (the opQueue and applyQueue run
   * independently), resurrecting `prev` here would briefly un-delete a row
   * the server says is gone — healed only at the next reconcile. So when a
   * server tombstone is present we drop the row and let the canonical
   * state stand; the failed mutation's own optimistic fence is cleared
   * regardless so a later legitimate re-create of the id isn't blocked.
   */
  restoreRow(entity: string, id: string, prev: Row | null): void {
    // The failed mutation's own optimistic fence always clears.
    this.optimisticTombstones.get(entity)?.delete(id);
    if (this.tombstones.get(entity)?.has(id)) {
      // Server authoritatively removed this row mid-flight — its
      // deletion outranks our local rollback.
      this.tables.get(entity)?.delete(id);
      this.notify();
      return;
    }
    if (prev) {
      if (!this.tables.has(entity)) this.tables.set(entity, new Map());
      this.tables.get(entity)!.set(id, prev);
    } else {
      this.tables.get(entity)?.delete(id);
    }
    this.notify();
  }

  /** Apply an optimistic delete. Block any incoming insert/update
   *  for this id until the server's authoritative delete arrives. */
  optimisticDelete(entity: string, id: string): void {
    this.tables.get(entity)?.delete(id);
    if (!this.optimisticTombstones.has(entity)) {
      this.optimisticTombstones.set(entity, new Set());
    }
    this.optimisticTombstones.get(entity)!.add(id);
    this.notify();
  }

  /**
   * Apply a reconcile result against the local replica. Reconcile is
   * "the server says these are the canonical rows right now" — it does
   * NOT carry per-event seqs. Distinct from `applyChange` (which
   * threads server-issued change events through a monotonic seq
   * guard + cursor advance).
   *
   * `tombstoneSeq` is the cursor at the start of the reconcile fetch.
   * - Upserts skip rows whose tombstone is fresher (would be stale
   *   resurrections of a later delete).
   * - Removals tombstone at `tombstoneSeq` so a later legitimate
   *   re-create (with a strictly greater seq) flows through.
   *
   * Cursor is NOT advanced — reconcile fabricates no real seqs, so
   * the engine's cursor stays pinned to the change-log position the
   * reconcile started from.
   */
  async applyReconcileBatch(
    entity: string,
    upserts: Row[],
    removalIds: string[],
    tombstoneSeq: number,
  ): Promise<void> {
    if (!this.tables.has(entity)) this.tables.set(entity, new Map());
    const table = this.tables.get(entity)!;
    const applied: Array<{ id: string; row: Row }> = [];
    for (const row of upserts) {
      const id = (row as { id?: unknown }).id;
      if (typeof id !== "string" || id.length === 0) continue;
      // Treat the upsert as effective at `tombstoneSeq + 1`. Anything
      // tombstoned at a HIGHER seq is fresher and wins; the upsert is
      // a stale view that pre-dated the delete.
      if (this.isTombstoned(entity, id, tombstoneSeq + 1)) continue;
      const merged = { ...row, id };
      table.set(id, merged);
      applied.push({ id, row: merged });
    }
    const removed: string[] = [];
    for (const id of removalIds) {
      if (this.reconcileRemove(entity, id, tombstoneSeq)) {
        removed.push(id);
      }
    }
    if (applied.length > 0 || removed.length > 0) this.notify();
    // Persist sequentially so disk order matches memory order — same
    // discipline as applyChangesAsync.
    if (this._persistFn) {
      for (const { id, row } of applied) {
        const ev: ChangeEvent = {
          seq: tombstoneSeq + 1,
          entity,
          row_id: id,
          kind: "insert",
          data: row,
          timestamp: "",
        };
        const result = this._persistFn(ev);
        if (result instanceof Promise) await result;
      }
      for (const id of removed) {
        const ev: ChangeEvent = {
          seq: tombstoneSeq,
          entity,
          row_id: id,
          kind: "delete",
          data: undefined as unknown as Row,
          timestamp: "",
        };
        const result = this._persistFn(ev);
        if (result instanceof Promise) await result;
      }
    }
  }

  /**
   * Drop every table + tombstone in-place, then notify. Used by the
   * sync engine's `resetReplica()` on identity flip (token or tenant
   * changed — the old replica reflects a different visible set).
   */
  clearAll(): void {
    this.tables.clear();
    this.tombstones.clear();
    this.optimisticTombstones.clear();
    this.notify();
  }
}
