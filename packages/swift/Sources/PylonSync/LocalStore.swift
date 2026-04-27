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
        guard let tombSeq = tombstones[entity]?[id] else { return false }
        return atSeq < tombSeq
    }

    private func recordTombstoneLocked(entity: String, id: String, seq: Int64) {
        if tombstones[entity] == nil { tombstones[entity] = [:] }
        let existing = tombstones[entity]?[id]
        if existing == nil || seq > (existing ?? 0) {
            tombstones[entity]?[id] = seq
        }
    }

    // MARK: - Optimistic

    /// Apply an optimistic insert. Returns a temporary id the caller should
    /// pass back to the mutation queue.
    @discardableResult
    public func optimisticInsert(_ entity: String, _ data: Row) -> String {
        let tempId = "_pending_\(Int(Date().timeIntervalSince1970 * 1000))_\(UUID().uuidString.prefix(8))"
        lock.lock()
        if tables[entity] == nil { tables[entity] = [:] }
        var copy = data
        copy["id"] = .string(tempId)
        tables[entity]?[tempId] = copy
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
        return tempId
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

    public func optimisticDelete(_ entity: String, id: String) {
        lock.lock()
        tables[entity]?.removeValue(forKey: id)
        // Use Int64.max so the next server replay can't undo this delete
        // until the authoritative event refreshes the tombstone.
        recordTombstoneLocked(entity: entity, id: id, seq: .max)
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
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
        let listeners = Array(self.listeners.values)
        lock.unlock()
        for l in listeners { l() }
    }
}
