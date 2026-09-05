import Foundation
import PylonClient

public typealias Row = [String: JSONValue]

/// In-memory replica of server state. Mirrors `LocalStore` from
/// `packages/sync/src/index.ts` — same tombstone semantics, same
/// authoritative-id handling on insert/update, same notify model.
///
/// Thread-safe via an internal lock. Listeners are invoked on the caller
/// thread that triggered the change — wrap in `Task { @MainActor in ... }`
/// when bridging to UI frameworks if you need main-thread guarantees.
public final class LocalStore: @unchecked Sendable {
    private let lock = NSLock()
    private var tables: [String: [String: Row]] = [:]

    /// Tombstones: `(entity, row_id) -> deletedAt seq`. A row whose id is
    /// here is considered deleted; any insert/update with `seq < tombSeq`
    /// is dropped so a delayed replay can't resurrect it.
    private var tombstones: [String: [String: Int64]] = [:]
    /// Rows deleted optimistically and not yet confirmed by the server.
    /// Blocks an insert/update for the id until the server's own delete
    /// event lands (which records a real tombstone and clears this entry)
    /// or the push is rejected (`restoreRow` clears it). Kept apart from
    /// the seq-keyed tombstones so a client fence never outranks a later
    /// legitimate re-create of the id. Mirrors the TS `optimisticTombstones`.
    private var optimisticTombstones: [String: Set<String>] = [:]

    private var listeners: [UUID: @Sendable () -> Void] = [:]

    /// Async persistence callback. Called from `applyChanges` /
    /// `applyChangesAsync` after merging into memory. Use to write through
    /// to SQLite or another durable store.
    public var persistFn: (@Sendable (ChangeEvent) async -> Void)?

    public init() {}

    // MARK: - Reads

    public func list(_ entity: String) -> [Row] {
        lock.lock(); defer { lock.unlock() }
        guard let table = tables[entity] else { return [] }
        return Array(table.values)
    }

    public func get(_ entity: String, id: String) -> Row? {
        lock.lock(); defer { lock.unlock() }
        return tables[entity]?[id]
    }

    // MARK: - Apply

    /// Apply a single change event. Tombstone-aware: insert/update events
    /// older than a recorded delete are silently dropped.
    public func applyChange(_ change: ChangeEvent) {
        lock.lock(); defer { lock.unlock() }
        applyChangeLocked(change)
    }

    /// Apply many changes and notify once. Persistence runs fire-and-forget
    /// — prefer `applyChangesAsync(_:)` if you'll advance a sync cursor
    /// after this call.
    public func applyChanges(_ changes: [ChangeEvent]) {
        lock.lock()
        for change in changes { applyChangeLocked(change) }
        let snapshot = changes.map { hydrateFromMemoryLocked($0) }
        let listeners = Array(self.listeners.values)
        let persist = persistFn
        lock.unlock()

        for l in listeners { l() }
        if let persist {
            for change in snapshot {
                Task.detached { await persist(change) }
            }
        }
    }

    /// Apply many changes and `await` the persistence writes before
    /// returning. Use this whenever you'll save a cursor after — otherwise
    /// a crash between memory-apply and disk-write can persist a cursor
    /// ahead of the on-disk replica.
    public func applyChangesAsync(_ changes: [ChangeEvent]) async {
        // The lock is acquired and released in a synchronous helper so the
        // compiler doesn't flag an NSLock call across an await boundary.
        let (snapshot, listeners, persist) = withLock {
            for change in changes { applyChangeLocked(change) }
            let snap = changes.map { hydrateFromMemoryLocked($0) }
            return (snap, Array(self.listeners.values), persistFn)
        }
        for l in listeners { l() }
        guard let persist else { return }
        await withTaskGroup(of: Void.self) { group in
            for change in snapshot {
                group.addTask { await persist(change) }
            }
        }
    }

    private func withLock<T>(_ body: () -> T) -> T {
        lock.lock(); defer { lock.unlock() }
        return body()
    }

    private func applyChangeLocked(_ change: ChangeEvent) {
        if tables[change.entity] == nil {
            tables[change.entity] = [:]
        }
        if (change.kind == .insert || change.kind == .update),
           isTombstonedLocked(entity: change.entity, id: change.row_id, atSeq: change.seq) {
            return
        }
        switch change.kind {
        case .insert:
            guard let data = change.data else { return }
            // Spread first, then force authoritative id — mirrors the TS
            // safety against a buggy server event corrupting the primary key.
            var merged = data
            merged["id"] = .string(change.row_id)
            tables[change.entity]?[change.row_id] = merged
        case .update:
            guard let data = change.data else { return }
            var existing = tables[change.entity]?[change.row_id] ?? ["id": .string(change.row_id)]
            for (k, v) in data { existing[k] = v }
            existing["id"] = .string(change.row_id)
            tables[change.entity]?[change.row_id] = existing
        case .delete:
            tables[change.entity]?.removeValue(forKey: change.row_id)
            recordTombstoneLocked(entity: change.entity, id: change.row_id, seq: change.seq)
        }
    }

    private func hydrateFromMemoryLocked(_ change: ChangeEvent) -> ChangeEvent {
        if change.kind == .delete { return change }
        guard let merged = tables[change.entity]?[change.row_id] else { return change }
        var copy = change
        copy.data = merged
        return copy
    }

    private func isTombstonedLocked(entity: String, id: String, atSeq: Int64) -> Bool {
        if optimisticTombstones[entity]?.contains(id) == true { return true }
        guard let tombSeq = tombstones[entity]?[id] else { return false }
        return atSeq < tombSeq
    }

    private func recordTombstoneLocked(entity: String, id: String, seq: Int64) {
        // A server-issued tombstone supersedes the optimistic fence for
        // this id, so a later legitimate re-create is not blocked.
        optimisticTombstones[entity]?.remove(id)
        if tombstones[entity] == nil { tombstones[entity] = [:] }
        let existing = tombstones[entity]?[id]
        if existing == nil || seq > (existing ?? 0) {
            tombstones[entity]?[id] = seq
        }
    }

    // MARK: - Optimistic

    /// Apply an optimistic insert under a freshly minted Pylon id and
    /// return it. The engine threads the same id through the push so the
    /// canonical server row replaces the ghost in place.
    @discardableResult
    public func optimisticInsert(_ entity: String, _ data: Row) -> String {
        let id = PylonIds.generate()
        optimisticInsertWithId(entity, id: id, data)
        return id
    }

    /// Apply an optimistic insert under a caller-chosen id. Mirrors the TS
    /// `optimisticInsertWithId`.
    public func optimisticInsertWithId(_ entity: String, id: String, _ data: Row) {
        lock.lock()
        if tables[entity] == nil { tables[entity] = [:] }
        var copy = data
        copy["id"] = .string(id)
        tables[entity]?[id] = copy
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
    }

    public func optimisticUpdate(_ entity: String, id: String, _ data: Row) {
        lock.lock()
        guard var existing = tables[entity]?[id] else {
            lock.unlock(); return
        }
        for (k, v) in data { existing[k] = v }
        tables[entity]?[id] = existing
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
    }

    /// Apply an optimistic delete: drop the row and fence the id against
    /// any insert/update until the server's own delete event arrives (or
    /// the push is rejected and `restoreRow` lifts the fence). The fence is
    /// not a seq tombstone: a server re-create of the id after the delete
    /// still lands once the server's delete has cleared it.
    public func optimisticDelete(_ entity: String, id: String) {
        lock.lock()
        tables[entity]?.removeValue(forKey: id)
        if optimisticTombstones[entity] == nil { optimisticTombstones[entity] = [] }
        optimisticTombstones[entity]?.insert(id)
        let listeners = Array(self.listeners.values)
        let persist = persistFn
        lock.unlock()
        for l in listeners { l() }
        // Mirror applyChange's persistence path so the delete survives
        // an app restart. Without this, the row sits in memory as
        // deleted but the on-disk replica still has it — relaunching
        // restores LocalStore from disk and the row reappears. The
        // pending mutation in the queue re-pushes the delete on launch.
        if let persist {
            let change = ChangeEvent(
                seq: 0,
                entity: entity,
                row_id: id,
                kind: .delete,
                data: nil,
                timestamp: ""
            )
            Task.detached { await persist(change) }
        }
    }

    // MARK: - Listeners

    @discardableResult
    public func subscribe(_ listener: @escaping @Sendable () -> Void) -> () -> Void {
        let id = UUID()
        lock.lock()
        listeners[id] = listener
        lock.unlock()
        return { [weak self] in
            self?.lock.lock()
            self?.listeners.removeValue(forKey: id)
            self?.lock.unlock()
        }
    }

    /// Manually fire all listeners. Used by `SyncEngine` after side-channel
    /// updates (presence, resolved-session changes) so subscribers re-read.
    public func notify() {
        lock.lock()
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
    }

    /// Drop every table + tombstone in-place, then notify. Used on identity
    /// flip (token or tenant changed).
    public func clearAll() {
        lock.lock()
        tables.removeAll()
        tombstones.removeAll()
        optimisticTombstones.removeAll()
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
    }

    // MARK: - Reconcile + rollback (TS parity)
    //
    // These mirror `packages/sync/src/local-store.ts`. The SyncEngine uses
    // them to sweep phantom rows (rows the server no longer returns) and to
    // undo optimistic mutations the server permanently rejected — the two
    // behaviors the Swift engine was missing, which surfaced as the Mac app
    // showing stale/ghost recordings that never cleared.

    /// Entity names that currently have a local table. Reconcile unions this
    /// with the engine's observed-entity set so a server row in a
    /// never-cached entity is still swept in.
    public func entityNames() -> [String] {
        lock.lock(); defer { lock.unlock() }
        return Array(tables.keys)
    }

    /// Reconcile a freshly-fetched server snapshot for one entity: upsert the
    /// `upserts`, and remove+tombstone every id in `removalIds`. The engine
    /// has already excluded rows with pending/failed mutations from both
    /// lists. `tombstoneSeq` is the engine's cursor at fetch time, so a later
    /// re-creation (higher seq) bypasses the tombstone and re-adds still flow.
    public func applyReconcileBatch(
        _ entity: String,
        upserts: [Row],
        removalIds: [String],
        tombstoneSeq: Int64
    ) {
        let (snapshot, listeners, persist) = withLock {
            () -> ([ChangeEvent], [@Sendable () -> Void], (@Sendable (ChangeEvent) async -> Void)?) in
            if tables[entity] == nil { tables[entity] = [:] }
            var snaps: [ChangeEvent] = []
            for row in upserts {
                guard case let .string(id)? = row["id"], !id.isEmpty else { continue }
                // A row tombstoned at a HIGHER seq is fresher — the upsert is
                // a stale pre-delete view; skip it.
                if isTombstonedLocked(entity: entity, id: id, atSeq: tombstoneSeq + 1) { continue }
                var merged = row
                merged["id"] = .string(id)
                tables[entity]?[id] = merged
                snaps.append(ChangeEvent(seq: tombstoneSeq, entity: entity, row_id: id, kind: .insert, data: merged, timestamp: ""))
            }
            for id in removalIds {
                if tables[entity]?.removeValue(forKey: id) != nil {
                    recordTombstoneLocked(entity: entity, id: id, seq: tombstoneSeq)
                    snaps.append(ChangeEvent(seq: tombstoneSeq, entity: entity, row_id: id, kind: .delete, data: nil, timestamp: ""))
                }
            }
            return (snaps, Array(self.listeners.values), persistFn)
        }
        guard !snapshot.isEmpty else { return }
        for l in listeners { l() }
        if let persist {
            for change in snapshot { Task.detached { await persist(change) } }
        }
    }

    /// Per-subscriber row revocation: the server signaled that this client
    /// lost read access to one row. ALWAYS records the tombstone (even when
    /// the row is not in memory) so a stale insert/update that arrives after
    /// the revocation cannot resurrect it. Returns whether a row was removed.
    /// Mirrors the TS `revokeRow`.
    public func revokeRow(_ entity: String, id: String, tombstoneSeq: Int64) -> Bool {
        lock.lock()
        let removed = tables[entity]?.removeValue(forKey: id) != nil
        recordTombstoneLocked(entity: entity, id: id, seq: tombstoneSeq)
        let listeners = removed ? Array(self.listeners.values) : []
        lock.unlock()
        for l in listeners { l() }
        return removed
    }

    /// Remove an optimistic insert ghost whose push was permanently rejected.
    /// No tombstone — the id was never real server state.
    public func rollbackOptimisticInsert(_ entity: String, id: String) {
        lock.lock()
        let removed = tables[entity]?.removeValue(forKey: id) != nil
        let listeners = removed ? Array(self.listeners.values) : []
        lock.unlock()
        for l in listeners { l() }
    }

    /// Restore a row to its captured pre-mutation value after a PERMANENTLY-
    /// rejected update/delete, clearing the optimistic-delete fence so the
    /// row (and future server events for it) aren't blocked. Only called when
    /// the server rejected the op (4xx) — so the row is unchanged server-side
    /// and the restore is authoritative. `prev == nil` removes the row (the
    /// pre-mutation state was "absent"). Persists the restored value so it
    /// survives the optimistic delete's on-disk tombstone.
    public func restoreRow(_ entity: String, id: String, prev: Row?) {
        let (snapshot, listeners, persist) = withLock {
            () -> (ChangeEvent, [@Sendable () -> Void], (@Sendable (ChangeEvent) async -> Void)?) in
            // The failed mutation's own optimistic fence always clears.
            optimisticTombstones[entity]?.remove(id)
            let snap: ChangeEvent
            if tombstones[entity]?[id] != nil {
                // The server removed this row while the rejected push was in
                // flight; its delete outranks the local rollback.
                tables[entity]?.removeValue(forKey: id)
                snap = ChangeEvent(seq: 0, entity: entity, row_id: id, kind: .delete, data: nil, timestamp: "")
            } else if let prev {
                if tables[entity] == nil { tables[entity] = [:] }
                var row = prev
                row["id"] = .string(id)
                tables[entity]?[id] = row
                snap = ChangeEvent(seq: 0, entity: entity, row_id: id, kind: .insert, data: row, timestamp: "")
            } else {
                tables[entity]?.removeValue(forKey: id)
                snap = ChangeEvent(seq: 0, entity: entity, row_id: id, kind: .delete, data: nil, timestamp: "")
            }
            return (snap, Array(self.listeners.values), persistFn)
        }
        for l in listeners { l() }
        if let persist { Task.detached { await persist(snapshot) } }
    }
}
