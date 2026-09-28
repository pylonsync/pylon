import Foundation
import PylonClient

/// Thrown by `SyncEngine.insert` / `update` / `delete` when the server
/// rejects the write (policy denial, validation, conflict). The optimistic
/// change is rolled back before the call throws. Parity with the TS
/// `MutationRejectedError`.
///
/// `code` is the server's error code when it sent one (`POLICY_DENIED`,
/// `VALIDATION_FAILED`, ...). `DISCARDED` means the write never reached the
/// server: an identity change dropped it from the queue.
public struct MutationRejectedError: Error, Sendable, Equatable {
    public let code: String
    public let message: String
    public let opId: String
    public let entity: String
    public let rowId: String
    public let kind: ChangeKind

    public init(code: String?, message: String, opId: String, entity: String, rowId: String, kind: ChangeKind) {
        self.code = code ?? "REJECTED"
        self.message = message
        self.opId = opId
        self.entity = entity
        self.rowId = rowId
        self.kind = kind
    }
}

public struct PendingMutation: Sendable, Codable, Hashable {
    public var id: String
    public var change: ClientChange
    public var status: Status
    public var error: String?
    /// Server error code for a failed mutation, when the server sent one.
    public var errorCode: String?
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
    /// Callers waiting for a mutation's outcome, by op id.
    private var waiters: [String: [CheckedContinuation<Void, Error>]] = [:]
    /// Mutations added with `trackOutcome: true` whose caller has not
    /// called `waitForOutcome` yet, with the outcome if it already landed.
    /// Lets the caller push first and wait after: the push can apply the
    /// mutation and prune it from the queue before the wait starts.
    private var tracked: [String: Result<Void, Error>?] = [:]

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
    ///
    /// `trackOutcome: true` records the outcome until `waitForOutcome(id)`
    /// is called, so the caller can push before it waits. Every tracked id
    /// must be waited on once.
    @discardableResult
    public func add(_ change: ClientChange, prevRow: Row? = nil, trackOutcome: Bool = false) async -> String {
        let id = "mut_\(Int(Date().timeIntervalSince1970 * 1000))_\(UUID().uuidString.prefix(8))"
        var changeWithOp = change
        changeWithOp.op_id = id
        queue.append(PendingMutation(id: id, change: changeWithOp, prevRow: prevRow))
        if trackOutcome { tracked[id] = .some(nil) }
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

    /// Return once the server applies the mutation or the mutation is
    /// queued behind a transient failure (`settleQueued`); throw a
    /// `MutationRejectedError` once the server rejects it. Returns or
    /// throws at once when the mutation already reached a terminal state.
    public func waitForOutcome(_ id: String) async throws {
        if let entry = tracked.removeValue(forKey: id), let result = entry {
            try result.get()
            return
        }
        if let m = queue.first(where: { $0.id == id }) {
            if m.status == .applied { return }
            if m.status == .failed { throw Self.rejection(for: m) }
        }
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            waiters[id, default: []].append(c)
        }
    }

    /// Release the waiters of a mutation that stays queued after a
    /// transient push failure (offline, 5xx). The mutation stays pending
    /// and is retried; a later rejection shows up as `status == .failed`.
    public func settleQueued(_ id: String) {
        resumeWaiters(id, with: .success(()))
    }

    public func markApplied(_ id: String) async {
        if let idx = queue.firstIndex(where: { $0.id == id }) {
            queue[idx].status = .applied
        }
        await flush()
        resumeWaiters(id, with: .success(()))
    }

    public func markFailed(_ id: String, error: String, code: String? = nil) async {
        var failed: PendingMutation?
        if let idx = queue.firstIndex(where: { $0.id == id }) {
            queue[idx].status = .failed
            queue[idx].error = error
            queue[idx].errorCode = code
            failed = queue[idx]
        }
        await flush()
        let err = failed.map(Self.rejection(for:)) ?? MutationRejectedError(
            code: code, message: error, opId: id, entity: "", rowId: "", kind: .insert)
        resumeWaiters(id, with: .failure(err))
    }

    private func resumeWaiters(_ id: String, with result: Result<Void, Error>) {
        if tracked[id] != nil, waiters[id] == nil {
            // First outcome wins; the caller reads it in waitForOutcome.
            if case .some(.none) = tracked[id] { tracked[id] = .some(result) }
            return
        }
        guard let list = waiters.removeValue(forKey: id) else { return }
        for c in list { c.resume(with: result) }
    }

    private static func rejection(for m: PendingMutation) -> MutationRejectedError {
        MutationRejectedError(
            code: m.errorCode,
            message: m.error ?? "The server rejected the write.",
            opId: m.id,
            entity: m.change.entity,
            rowId: m.change.row_id,
            kind: m.change.kind
        )
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
        let dropped = queue
        queue.removeAll()
        await flush()
        // Nobody will push these writes any more. Fail their waiters so a
        // caller awaiting the outcome does not hang.
        for m in dropped {
            let err = MutationRejectedError(
                code: "DISCARDED",
                message: "The write was discarded because the signed-in identity changed.",
                opId: m.id,
                entity: m.change.entity,
                rowId: m.change.row_id,
                kind: m.change.kind
            )
            resumeWaiters(m.id, with: .failure(err))
        }
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
