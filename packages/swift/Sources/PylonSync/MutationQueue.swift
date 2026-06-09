import Foundation
import PylonClient

public struct PendingMutation: Sendable, Codable, Hashable {
    public var id: String
    public var change: ClientChange
    public var status: Status
    public var error: String?
    /// Pre-mutation snapshot of the row, captured at queue time for
    /// update/delete. Used to restore the row if the push is permanently
    /// rejected (see `SyncEngine.failPushedMutation`). `nil` for inserts and
    /// for updates whose target was itself an un-acked insert.
    public var prevRow: Row?

    public enum Status: String, Sendable, Codable, Hashable {
        case pending
        case applied
        case failed
    }

    public init(
        id: String,
        change: ClientChange,
        status: Status = .pending,
        error: String? = nil,
        prevRow: Row? = nil
    ) {
        self.id = id
        self.change = change
        self.status = status
        self.error = error
        self.prevRow = prevRow
    }
}

/// Persistence backend for the mutation queue. Implement against SQLite,
/// the filesystem, or any KV store. The default sync engine swaps in
/// `SQLiteMutationPersistence` from this module.
public protocol MutationQueuePersistence: Sendable {
    func saveAll(_ mutations: [PendingMutation]) async throws
    func loadAll() async throws -> [PendingMutation]
}

/// Offline-safe write queue. Mutations are minted with a stable `op_id`
/// that doubles as the server-side idempotency key — replays on retry are
/// short-circuited server-side.
///
/// Mirrors `MutationQueue` from `packages/sync/src/index.ts`. Failed
/// mutations are kept (not dropped) so the UI can surface them to the
/// user. Applied mutations are pruned by `clear()`.
public actor MutationQueue {
    private var queue: [PendingMutation] = []
    private var persistence: MutationQueuePersistence?

    public init(persistence: MutationQueuePersistence? = nil) {
        self.persistence = persistence
    }

    public func attachPersistence(_ p: MutationQueuePersistence) {
        self.persistence = p
    }

    /// Load any persisted mutations from the backend. Call once at startup.
    public func hydrate() async {
        guard let persistence else { return }
        do {
            let loaded = try await persistence.loadAll()
            let existing = Set(queue.map(\.id))
            var mergedAny = false
            for m in loaded where !existing.contains(m.id) {
                queue.append(m)
                mergedAny = true
            }
            if mergedAny { await flush() }
        } catch {
            // Broken storage shouldn't take down the app — degrade to
            // memory-only mode silently. App can inspect logs to diagnose.
        }
    }

    /// Append a mutation. Returns the `op_id` for caller bookkeeping.
    /// `prevRow` is the pre-mutation row snapshot (update/delete) used to
    /// roll back on a permanent push rejection.
    @discardableResult
    public func add(_ change: ClientChange, prevRow: Row? = nil) async -> String {
        let id = "mut_\(Int(Date().timeIntervalSince1970 * 1000))_\(UUID().uuidString.prefix(8))"
        var changeWithOp = change
        changeWithOp.op_id = id
        queue.append(PendingMutation(id: id, change: changeWithOp, prevRow: prevRow))
        await flush()
        return id
    }

    public func pending() -> [PendingMutation] {
        queue.filter { $0.status == .pending }
    }

    public func all() -> [PendingMutation] { queue }

    /// `"{entity}/{row_id}"` keys for every pending OR failed mutation.
    /// Reconcile must NOT sweep or overwrite these rows — an offline insert
    /// that hasn't pushed yet would otherwise look like a phantom local row
    /// and get tombstoned before it ships. Mirrors TS `pendingRowKeys()`.
    public func pendingRowKeys() -> Set<String> {
        var keys = Set<String>()
        for m in queue where m.status == .pending || m.status == .failed {
            keys.insert("\(m.change.entity)/\(m.change.row_id)")
        }
        return keys
    }

    public func markApplied(_ id: String) async {
        if let idx = queue.firstIndex(where: { $0.id == id }) {
            queue[idx].status = .applied
        }
        await flush()
    }

    public func markFailed(_ id: String, error: String) async {
        if let idx = queue.firstIndex(where: { $0.id == id }) {
            queue[idx].status = .failed
            queue[idx].error = error
        }
        await flush()
    }

    /// Drop applied mutations. Failed ones are kept so the UI can ack/retry.
    public func clear() async {
        queue.removeAll { $0.status == .applied }
        await flush()
    }

    /// Drop EVERY queued mutation (pending + failed) and clear the persisted
    /// copy. Used on an identity flip: the outgoing identity's un-pushed
    /// writes must NOT be replayed under the incoming identity's token — the
    /// "cross-identity write leak" the TS client's `wipeMutations` prevents.
    public func wipeAll() async {
        queue.removeAll()
        await flush()
    }

    public func remove(_ id: String) async {
        queue.removeAll { $0.id == id }
        await flush()
    }

    private func flush() async {
        guard let persistence else { return }
        let snapshot = queue
        do {
            try await persistence.saveAll(snapshot)
        } catch {
            // Log + drop — see comment on hydrate(). The next mutation will
            // re-attempt the write.
        }
    }
}
