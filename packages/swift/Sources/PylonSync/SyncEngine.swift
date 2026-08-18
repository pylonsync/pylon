import Foundation
import PylonClient

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public struct SyncEngineConfig: Sendable {
    public var baseURL: URL
    /// Override for the WebSocket URL. Falls back to a derivation from
    /// `baseURL` (port + 1 if explicit port is set, else same hostname on
    /// `wss`/`ws`). Set this when the WebSocket is exposed on a different
    /// host or port (e.g. behind a separate Fly service).
    public var wsURL: URL?
    /// Override for the SSE endpoint. Falls back to `baseURL` host with
    /// port + 2 and path `/events` (matches `pylon dev` convention).
    public var sseURL: URL?
    public var transport: TransportType
    public var pollInterval: TimeInterval
    /// Base delay for the exponential backoff, in seconds. Default 1s.
    public var reconnectBaseDelay: TimeInterval
    public var appName: String
    /// Keep the local replica across ORG switches (same user). Default
    /// `true`: a tenant flip wipes the replica and re-bootstraps, because
    /// tenant-scoped read policies mean the visible set genuinely changed.
    /// Set `false` ONLY when every synced entity's read policy is
    /// MEMBERSHIP-scoped (`exists(OrgMember ...)`): then the replica is
    /// valid across all the user's orgs, and an org switch keeps every
    /// row and reconciles in the background instead of wiping. USER flips
    /// (login/logout) always wipe regardless. Parity with the TS engine's
    /// `resetOnTenantFlip`.
    public var resetOnTenantFlip: Bool
    /// Connect the live-event socket through the Durable Object sync
    /// relay instead of the machine's own WS. The engine fetches a
    /// signed connect target from `GET /api/sync/relay-token` before
    /// each connect (fresh token every reconnect). Pull/push/mutations
    /// still go to `baseURL`; the relay carries change events only —
    /// room, CRDT, and reactive subscriptions need the direct machine
    /// WS. Parity with the TS engine's `relay` config.
    public var relay: Bool

    public enum TransportType: String, Sendable {
        case websocket
        case sse
        case poll
    }

    public init(
        baseURL: URL,
        wsURL: URL? = nil,
        sseURL: URL? = nil,
        transport: TransportType = .websocket,
        pollInterval: TimeInterval = 1.0,
        reconnectBaseDelay: TimeInterval = 1.0,
        appName: String = "default",
        resetOnTenantFlip: Bool = true,
        relay: Bool = false
    ) {
        self.baseURL = baseURL
        self.wsURL = wsURL
        self.sseURL = sseURL
        self.transport = transport
        self.pollInterval = pollInterval
        self.reconnectBaseDelay = reconnectBaseDelay
        self.appName = appName
        self.resetOnTenantFlip = resetOnTenantFlip
        self.relay = relay
    }
}

/// Coordinates pull, push, local store, mutation queue, and the realtime
/// transport. Mirrors `SyncEngine` from `packages/sync/src/index.ts` —
/// same wire formats, same identity-flip detection, same circuit breakers.
///
/// Call `start()` to boot, `stop()` to tear down. Insert/update/delete
/// methods write optimistically and replicate via the queue. Subscribe via
/// `store.subscribe(_:)` to react to changes.
public actor SyncEngine {
    public let config: SyncEngineConfig
    public let store: LocalStore
    public let mutations: MutationQueue
    public let client: PylonClient

    /// Stable per-client identifier. Persisted via the storage adapter so
    /// reloads get the same id.
    public let clientId: String

    private(set) var cursor: SyncCursor = SyncCursor()

    private var running = false
    private var ws: PylonWebSocket?
    private var reconnectAttempts = 0
    private var consecutive410s = 0
    /// Live-event hold buffer, non-nil while a pull is in flight (parity
    /// with the TS engine's `pullHold`). A live WS/SSE frame landing
    /// mid-pull used to apply immediately and jump the cursor past the
    /// not-yet-pulled gap, so the pull's own events were then dropped by
    /// the seq gate — silently missing rows until a reconcile swept them.
    /// Held frames replay (seq-gated) after the pull lands; on a failed
    /// pull they're discarded — the cursor never reached their seqs, so
    /// the next pull re-fetches them from the log.
    private var pullHold: [ChangeEvent]? = nil
    private var lastSeenToken: String? = nil
    private var lastSeenTokenObserved = false
    private var lastSeenTenant: String? = nil
    private var lastSeenTenantObserved = false
    private var resolvedSession = ResolvedSession()
    private var presenceData: [String: JSONValue] = [:]
    private var crdtSubscribers: [String: Int] = [:]
    private var crdtSubscriptions: Set<String> = []
    private var binaryHandlers: [UUID: @Sendable (Data) -> Void] = [:]
    private var pollTask: Task<Void, Never>?
    private var receiveTask: Task<Void, Never>?
    private var inFlightPush: Task<Void, Never>?
    private var stableTimer: Task<Void, Never>?
    private var persistence: SyncPersistence?
    private var sseStream: SSEStream?
    private var sseTask: Task<Void, Never>?
    private var wsConnected = false

    // Reconcile state (TS parity). `observedEntities` are entities the UI has
    // queried (even if empty locally) so reconcile sweeps server rows in a
    // never-cached entity. `hydrated` flips true after the first pull, so a
    // scoped reconcile only fires once data has loaded. `lastReconcileAt`
    // debounces no-arg reconciles; `reconcileInFlight` coalesces concurrent
    // ones. `lastPullStartedFromZero` records whether the last pull was a
    // cold snapshot (skip the post-pull reconcile then — the snapshot IS
    // authoritative).
    private var observedEntities: Set<String> = []
    private var hydrated = false
    private var lastReconcileAt: Date = .distantPast
    private var reconcileInFlight = false
    private var lastPullStartedFromZero = false

    // Initial-sync loading signal (TS parity — `_initialSyncSettled` in
    // packages/sync/src/index.ts). A query's `loading` must stay true through
    // the window where the local cache has hydrated (possibly EMPTY) but the
    // first SERVER pull hasn't landed yet — otherwise a cold launch, or a
    // post-`resetReplica` org switch, flashes the empty state for the seconds
    // the snapshot takes. `isInitialSyncSettled()` flips true after the first
    // successful pull settles; a 12s fallback guarantees it can never pin
    // (offline, or an empty entity that never receives a pull). Reset to false
    // on `resetReplica` so an org switch re-enters loading until the re-pull
    // lands.
    private var _initialSyncSettled = false
    private var initialSyncFallback: Task<Void, Never>?

    /// True after the first server pull settles (or the fallback fires). Drives
    /// `PylonQuery.loading` — mirrors `SyncEngine.isInitialSyncSettled()` in TS.
    public func isInitialSyncSettled() -> Bool { _initialSyncSettled }

    /// Flip the signal true (idempotent) + notify observers so a `PylonQuery`
    /// re-reads and drops its `loading`. Cancels any armed fallback.
    private func markInitialSyncSettled() {
        initialSyncFallback?.cancel()
        initialSyncFallback = nil
        if _initialSyncSettled { return }
        _initialSyncSettled = true
        store.notify()
    }

    /// Safety net so `loading` never pins: settle after a deadline even if no
    /// pull lands (offline; an empty entity whose pull errors). The real pull
    /// settles it far sooner in the normal case. Re-armable — a replica wipe
    /// (org switch / identity flip) resets the signal and re-arms this.
    private func armInitialSyncFallback() {
        initialSyncFallback?.cancel()
        initialSyncFallback = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 12_000_000_000)
            if Task.isCancelled { return }
            await self?.markInitialSyncSettled()
        }
    }

    /// Optional WebSocket factory. Default is `URLSessionWebSocket`.
    private let webSocketFactory: @Sendable (URL, String?) -> PylonWebSocket

    public init(
        config: SyncEngineConfig,
        client: PylonClient,
        persistence: SyncPersistence? = nil,
        webSocketFactory: (@Sendable (URL, String?) -> PylonWebSocket)? = nil
    ) async {
        self.config = config
        self.client = client
        self.store = LocalStore()
        self.mutations = MutationQueue()
        self.persistence = persistence
        self.webSocketFactory = webSocketFactory ?? { url, token in
            URLSessionWebSocket(url: url, token: token)
        }
        let storage = await client.storage
        self.clientId = SyncEngine.resolveClientId(storage: storage)
    }

    private static func resolveClientId(storage: PylonStorage) -> String {
        if let existing = storage.get(StorageKeys.clientId), !existing.isEmpty {
            return existing
        }
        let fresh = "cl_" + UUID().uuidString.lowercased()
        storage.set(StorageKeys.clientId, value: fresh)
        return fresh
    }

    // MARK: - Lifecycle

    public func start() async {
        guard !running else { return }
        running = true

        if let persistence {
            do {
                let cached = try await persistence.loadAllRows()
                var hydrated = false
                for (entity, rows) in cached {
                    for row in rows {
                        if let id = row["id"]?.stringValue {
                            store.applyChange(ChangeEvent(
                                seq: 0,
                                entity: entity,
                                row_id: id,
                                kind: .insert,
                                data: row,
                                timestamp: ""
                            ))
                            hydrated = true
                        }
                    }
                }
                if hydrated { store.notify() }
                if let saved = try await persistence.loadCursor() {
                    cursor = saved
                }
                let local = persistence
                store.persistFn = { change in
                    try? await local.persist(change)
                }
                await mutations.attachPersistence(persistence)
                await mutations.hydrate()
            } catch {
                // Persistence init failures degrade to memory-only.
            }
        }

        await refreshResolvedSession()
        // Re-enter loading until the FIRST pull lands rows (or the fallback
        // fires). pull()'s success path flips the signal; this arms the safety
        // net so an offline launch (pull errors) still settles within 12s.
        armInitialSyncFallback()
        await pull()
        hydrated = true
        if let persistence {
            try? await persistence.saveCursor(cursor)
        }

        switch config.transport {
        case .websocket:
            await connectWs()
        case .sse:
            await connectSse()
        case .poll:
            startPolling()
        }
    }

    public func stop() {
        running = false
        ws?.close()
        ws = nil
        wsConnected = false
        receiveTask?.cancel()
        receiveTask = nil
        pollTask?.cancel()
        pollTask = nil
        stableTimer?.cancel()
        stableTimer = nil
        sseStream?.close()
        sseStream = nil
        sseTask?.cancel()
        sseTask = nil
        initialSyncFallback?.cancel()
        initialSyncFallback = nil
    }

    /// True when the WebSocket transport is currently connected. SSE/poll
    /// modes always report `false` here — they have no persistent socket.
    public var connected: Bool {
        wsConnected
    }

    // MARK: - Pull / Push

    /// Pull changes from the server. Detects identity flips (token /
    /// tenant) and resets the replica before pulling under the new
    /// identity.
    public func pull() async {
        let tokenNow = await client.currentToken()
        if lastSeenTokenObserved && lastSeenToken != tokenNow {
            // Identity flip → wipe the previous identity's rows AND queued
            // writes (don't push user A's offline mutations under B's token).
            await resetReplica(wipeMutations: true)
            Task { await self.refreshResolvedSession() }
        }
        lastSeenToken = tokenNow
        lastSeenTokenObserved = true

        // A cold pull (cursor == 0) drains a full snapshot — paginated by the
        // server via `snapshot_after` (NOT `has_more`), which the engine
        // previously ignored, rendering only the first page. Drain BOTH so a
        // fresh client gets a consistent snapshot. `since` stays fixed at the
        // entry cursor during snapshot pages; for the delta tail it advances.
        let startedFromZero = cursor.last_seq == 0
        let sinceParam = cursor.last_seq
        // Open the live-event hold for the pull's whole duration so a
        // racing WS/SSE frame can't leapfrog the cursor past rows this
        // pull hasn't delivered yet (see `pullHold`). The success path
        // drains + replays it; this backstop discards whatever remains
        // on a failed pull (the next pull re-fetches those events).
        pullHold = []
        defer { pullHold = nil }
        // Fetch/apply pipelining (parity with the TS engine): page N's
        // apply runs while page N+1 is in flight, so a multi-page
        // catch-up costs max(network, apply) per page instead of their
        // sum. Ordering holds because the next apply only STARTS after
        // the previous one finished, and the cursor only advances
        // inside the apply task, after its rows landed. The next
        // `since` comes from each RESPONSE's cursor (`nextSince`), not
        // the engine cursor — the apply is what advances the engine
        // cursor, and waiting on it is exactly what pipelining removes.
        //
        // Declared outside the do: on a mid-loop fetch error the
        // previous page's apply may still be in flight, and the catch
        // (410 reset → re-pull) MUST let it settle first — otherwise
        // the orphaned apply races the replica wipe and re-writes
        // stale rows into the fresh store.
        var pendingApply: Task<Void, Never>? = nil
        do {
            var snapshotAfter: String? = nil
            var nextSince = sinceParam
            var pages = 0
            while pages < 10_000 {
                pages += 1
                let since = snapshotAfter != nil ? sinceParam : nextSince
                let resp = try await client.syncPull(since: since, snapshotAfter: snapshotAfter)
                // Reset the 410 circuit breaker ONLY on a successful DELTA
                // pull — a snapshot success leaving the counter intact means
                // repeated resyncs escalate backoff instead of melting egress.
                if !startedFromZero { consecutive410s = 0 }
                if let pending = pendingApply { await pending.value }
                let changes = resp.changes
                let respCursor = resp.cursor
                // Task {} inherits the actor's isolation, so touching
                // `cursor` / `persistence` here is safe; the pull loop
                // and this task interleave only at suspension points
                // (the network await vs. the store await).
                pendingApply = Task {
                    if !changes.isEmpty {
                        await self.store.applyChangesAsync(changes)
                    }
                    if respCursor.last_seq > self.cursor.last_seq {
                        self.cursor = respCursor
                        if let persistence = self.persistence {
                            try? await persistence.saveCursor(respCursor)
                        }
                    }
                }
                // No-progress guard: a page that reports more but does
                // not advance the cursor would refetch itself forever.
                let advanced = respCursor.last_seq > nextSince
                nextSince = max(nextSince, respCursor.last_seq)
                if let sa = resp.snapshot_after {
                    snapshotAfter = sa // more snapshot pages — keep `since` fixed
                } else if resp.has_more && advanced {
                    snapshotAfter = nil // delta tail — `since` advances per response
                } else {
                    break
                }
            }
            if let pending = pendingApply { await pending.value }
            // Pull landed cleanly → replay the frames held during it, in
            // arrival order. Each one re-checks the seq gate against the
            // now-advanced cursor, so frames the pull already delivered
            // dedupe and genuinely newer ones apply. Nil FIRST so the
            // replay isn't re-held.
            if let held = pullHold {
                pullHold = nil
                for change in held {
                    await applyLiveEvent(change)
                }
            }
            lastPullStartedFromZero = startedFromZero
            // First successful pull settled — drop query `loading`. Idempotent:
            // a no-op on every pull after the first (until the next resetReplica
            // flips the signal back to false).
            markInitialSyncSettled()
        } catch let error as PylonError {
            // Settle any in-flight page apply before acting on the
            // error — the 410 path wipes the replica, and an apply
            // landing after the wipe would resurrect stale rows and
            // advance the cursor under the recursive re-pull.
            if let pending = pendingApply { await pending.value }
            switch error.httpStatus {
            case 429:
                reconnectAttempts += 3
            case 410:
                let attempt = consecutive410s
                consecutive410s += 1
                if attempt == 0 {
                    // Discard this failed episode's held frames before the
                    // recursive pull opens its own hold — the from-zero
                    // snapshot it runs covers everything the buffer held.
                    pullHold = nil
                    await resetReplica()
                    await pull()
                } else {
                    let delayMs = min(30_000, 1000 * (1 << min(attempt, 5)))
                    Task {
                        try? await Task.sleep(nanoseconds: UInt64(delayMs) * 1_000_000)
                        await self.pull()
                    }
                }
            default:
                break
            }
        } catch {
            // Other transport errors swallow — caller will retry on next
            // tick. Still settle an in-flight apply so its rows and
            // cursor advance land before the next pull reads the cursor.
            if let pending = pendingApply { await pending.value }
        }
    }

    // MARK: - Reconcile (phantom-row sweep)

    /// Stable signature of the resolved session. A reconcile whose fetch
    /// raced a tenant/user flip must NOT apply its result — rows fetched
    /// under the old tenant would tombstone rows visible under the new one
    /// (the "flashes data away on first load" bug).
    private func sessionSignature() -> String {
        let roles = resolvedSession.roles.sorted().joined(separator: ",")
        return "\(resolvedSession.userId ?? "")|\(resolvedSession.tenantId ?? "")|\(resolvedSession.isAdmin ? "1" : "0")|\(roles)"
    }

    /// Register an entity the UI is observing (via a query) so reconcile
    /// sweeps it even when it has no local rows yet. Fires a one-shot scoped
    /// reconcile when the entity is empty post-hydration — covers a query on
    /// a never-cached entity that should surface a server row.
    public func observeEntity(_ entity: String) async {
        if observedEntities.contains(entity) { return }
        observedEntities.insert(entity)
        if hydrated && store.list(entity).isEmpty {
            await reconcile([entity])
        }
    }

    /// Sweep phantom rows: fetch each entity's authoritative server set and
    /// remove any local row the server no longer returns (plus upsert any
    /// drifted row). `nil` sweeps all known + observed entities (debounced);
    /// an explicit list bypasses the debounce. Rows with pending/failed
    /// mutations are protected. Mirrors TS `reconcile()` — this is the
    /// behavior that clears recordings deleted on another surface / server-
    /// side, which the Swift engine previously never did.
    public func reconcile(_ entities: [String]? = nil) async {
        let minIntervalMs = 2000.0
        if entities == nil,
           Date().timeIntervalSince(lastReconcileAt) * 1000 < minIntervalMs {
            return
        }
        if reconcileInFlight { return }
        reconcileInFlight = true
        defer {
            reconcileInFlight = false
            lastReconcileAt = Date()
        }
        await reconcileInner(entities)
    }

    private func reconcileInner(_ entities: [String]?) async {
        let names = entities ?? Array(Set(store.entityNames()).union(observedEntities))
        if names.isEmpty { return }
        // Tombstone any local row the server omits at the current cursor, so
        // a future re-creation (higher seq) still flows through.
        let tombstoneSeq = cursor.last_seq
        let pendingKeys = await mutations.pendingRowKeys()
        for entity in names {
            let cursorBefore = cursor.last_seq
            let sessionBefore = sessionSignature()
            let serverRows: [Row]
            do {
                serverRows = try await fetchEntityRows(entity)
            } catch let err as PylonError {
                if err.httpStatus == 403 || err.httpStatus == 404 {
                    await dropEntity(entity, tombstoneSeq: tombstoneSeq, pendingKeys: pendingKeys)
                }
                continue
            } catch {
                continue // transient — next trigger retries
            }
            // Drift guards: a WS event advanced the cursor, or the session
            // flipped, during the fetch. Applying a now-stale snapshot would
            // clobber fresher state / tombstone newly-visible rows. Skip.
            if cursor.last_seq != cursorBefore { continue }
            if sessionSignature() != sessionBefore { continue }

            var serverIds = Set<String>()
            var upserts: [Row] = []
            for row in serverRows {
                guard case let .string(id)? = row["id"], !id.isEmpty else { continue }
                serverIds.insert(id)
                if pendingKeys.contains("\(entity)/\(id)") { continue }
                let local = store.get(entity, id: id)
                if local == nil || local! != row { upserts.append(row) }
            }
            var removalIds: [String] = []
            for local in store.list(entity) {
                guard case let .string(id)? = local["id"] else { continue }
                if pendingKeys.contains("\(entity)/\(id)") { continue }
                if !serverIds.contains(id) { removalIds.append(id) }
            }
            if upserts.isEmpty && removalIds.isEmpty { continue }
            store.applyReconcileBatch(entity, upserts: upserts, removalIds: removalIds, tombstoneSeq: tombstoneSeq)
        }
    }

    /// Fetch every row for an entity via cursor pagination (cap 20 pages =
    /// 20k rows; larger tables shouldn't be fully mirrored client-side).
    /// 1000/page matches the sync-pull batch limits — the server honors it
    /// only on replication fetches, and each page is a full round trip, so
    /// the page size directly divides sweep latency (parity with the TS
    /// engine's fetchEntityRows).
    private func fetchEntityRows(_ entity: String) async throws -> [Row] {
        var out: [Row] = []
        var after: String? = nil
        for _ in 0..<20 {
            // `replication: true` -> `sync=1`, which is what makes the entity's
            // sync scope apply. These rows land in the replica; an app read
            // through loadPage/InfiniteQuery deliberately stays unscoped.
            let page = try await client.listCursor(
                entity, after: after, limit: 1000, replication: true, as: Row.self)
            out.append(contentsOf: page.data)
            guard page.has_more, let next = page.next_cursor else { break }
            after = next
        }
        return out
    }

    /// Drop every (unprotected) local row for an entity that became
    /// unreadable (403, policy revoked) or removed (404) — keeping them
    /// around just leaks invisible state.
    private func dropEntity(_ entity: String, tombstoneSeq: Int64, pendingKeys: Set<String>) async {
        var removalIds: [String] = []
        for local in store.list(entity) {
            guard case let .string(id)? = local["id"] else { continue }
            if pendingKeys.contains("\(entity)/\(id)") { continue }
            removalIds.append(id)
        }
        if removalIds.isEmpty { return }
        store.applyReconcileBatch(entity, upserts: [], removalIds: removalIds, tombstoneSeq: tombstoneSeq)
    }

    /// Push pending mutations. Coalesces concurrent callers to a single
    /// in-flight push.
    public func push() async {
        if let inFlight = inFlightPush {
            await inFlight.value
            return
        }
        let task = Task { await self.pushInner() }
        inFlightPush = task
        await task.value
        inFlightPush = nil
    }

    private func pushInner() async {
        let pending = await mutations.pending()
        guard !pending.isEmpty else { return }
        let req = PushRequest(changes: pending.map(\.change), client_id: clientId)
        do {
            let resp = try await client.syncPush(req)
            if let results = resp.results, !results.isEmpty {
                // Per-op mapping by op_id — correct on a partial batch failure
                // (e.g. op 1 applied, op 2 rejected, op 3 applied). Positional
                // mapping misaligns those.
                var byOpId: [String: PushOpResult] = [:]
                for r in results { if let op = r.op_id { byOpId[op] = r } }
                for m in pending {
                    guard let r = byOpId[m.id] else { continue }
                    switch r.status {
                    case "applied", "replayed", "deduped":
                        await mutations.markApplied(m.id)
                    case "error":
                        await failPushedMutation(m, error: r.error?.message ?? "rejected")
                    default:
                        break // "pending" → leave queued, retry next push
                    }
                }
            } else {
                // Legacy positional format (servers < 0.3.188).
                for (i, m) in pending.enumerated() {
                    if i < resp.applied {
                        await mutations.markApplied(m.id)
                    } else {
                        let idx = i - resp.applied
                        let msg = idx < resp.errors.count ? resp.errors[idx] : "rejected"
                        await failPushedMutation(m, error: msg)
                    }
                }
            }
            await mutations.clear()
            // Catch-up pull: if the server applied past our cursor (server-
            // side defaults, linked-row side effects), pull NOW instead of
            // waiting for the WS rebroadcast (which may be slow or dropped).
            let maxApplied = resp.max_applied_seq ?? resp.cursor.last_seq
            if maxApplied > cursor.last_seq {
                Task { await self.pull() }
            }
        } catch let err as PylonError {
            // Permanent rejection (4xx) → roll back the optimistic ghosts and
            // stop retrying. Transient (5xx/429/offline/no-status) → leave
            // pending; the poll loop / next push retries.
            if isPermanentPushError(err.httpStatus) {
                let msg: String
                if case let .http(_, code, m) = err { msg = m ?? code ?? "rejected" } else { msg = "rejected" }
                for m in pending { await failPushedMutation(m, error: msg) }
                await mutations.clear()
            }
        } catch {
            // Network throw (no status) — transient. Leave pending.
        }
    }

    /// Roll back an optimistic mutation the server permanently rejected:
    /// remove the ghost (insert) or restore the captured pre-mutation row
    /// (update/delete), then mark the mutation failed so the UI can surface
    /// it. Without this a rejected edit leaves a ghost row forever.
    private func failPushedMutation(_ m: PendingMutation, error: String) async {
        switch m.change.kind {
        case .insert:
            store.rollbackOptimisticInsert(m.change.entity, id: m.change.row_id)
        case .update, .delete:
            store.restoreRow(m.change.entity, id: m.change.row_id, prev: m.prevRow)
        }
        await mutations.markFailed(m.id, error: error)
    }

    /// Permanent push errors must NOT retry: 400/403/404/409/422. A missing
    /// status (network throw) and 5xx/429/401/408 are transient → keep
    /// retrying with backoff.
    private func isPermanentPushError(_ status: Int?) -> Bool {
        guard let status else { return false }
        return [400, 403, 404, 409, 422].contains(status)
    }

    // MARK: - Optimistic mutations

    @discardableResult
    public func insert(_ entity: String, _ data: Row) async -> String {
        let tempId = store.optimisticInsert(entity, data)
        await mutations.add(ClientChange(entity: entity, row_id: tempId, kind: .insert, data: data))
        await push()
        return tempId
    }

    public func update(_ entity: String, id: String, _ data: Row) async {
        // Snapshot the row BEFORE the optimistic apply so a permanent
        // rejection can restore it.
        let prev = store.get(entity, id: id)
        store.optimisticUpdate(entity, id: id, data)
        await mutations.add(ClientChange(entity: entity, row_id: id, kind: .update, data: data), prevRow: prev)
        await push()
    }

    public func delete(_ entity: String, id: String) async {
        let prev = store.get(entity, id: id)
        store.optimisticDelete(entity, id: id)
        await mutations.add(ClientChange(entity: entity, row_id: id, kind: .delete), prevRow: prev)
        await push()
    }

    // MARK: - WebSocket

    private func deriveWsURL() -> URL {
        if let override = config.wsURL { return override }
        var components = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false)!
        let isHttps = components.scheme == "https"
        components.scheme = isHttps ? "wss" : "ws"
        components.path = ""
        if let port = components.port {
            // pylon dev convention: WS on port + 1.
            components.port = port + 1
        }
        return components.url ?? config.baseURL
    }

    /// Relay mode: `GET /api/sync/relay-token` → the relay ws URL (with
    /// the `since` cursor) plus the signed blob. `nil` on any failure
    /// (the caller backs off like a failed connect). The blob is
    /// returned separately, NOT baked into the URL — it travels in the
    /// `bearer.<blob>` subprotocol so the credential stays out of
    /// proxy/CDN access logs. Mirrors the TS transport's getRelayTarget.
    private func fetchRelayTarget() async -> (url: URL, blob: String)? {
        guard let minted = try? await client.syncRelayToken() else { return nil }
        let sep = minted.url.contains("?") ? "&" : "?"
        guard let url = URL(string: "\(minted.url)\(sep)since=\(cursor.last_seq)") else {
            return nil
        }
        return (url, minted.token)
    }

    private func connectWs() async {
        guard running else { return }
        // Re-auth before (re)opening the socket: detect a tenant/token change
        // that happened while it was closed (sign-out→in, org switch) so we
        // open with fresh credentials + a reset replica instead of streaming
        // the old identity's rows.
        await refreshResolvedSession()
        let url: URL
        let token: String?
        if config.relay {
            // Relay mode: mint a fresh signed connect target for THIS
            // attempt. The blob rides the bearer subprotocol (the
            // factory prefixes `bearer.`), so the machine session token
            // never reaches the relay and the blob never hits the URL.
            guard let target = await fetchRelayTarget() else {
                wsConnected = false
                scheduleReconnect()
                return
            }
            url = target.url
            token = target.blob
        } else {
            url = deriveWsURL()
            token = await client.currentToken()
        }
        let socket = webSocketFactory(url, token)
        ws = socket
        do {
            try await socket.connect()
            wsConnected = true
        } catch {
            wsConnected = false
            scheduleReconnect()
            return
        }
        // Sweep rows that changed while disconnected — the WS only delivers
        // LIVE events, so a delete/update that landed during the outage is
        // otherwise missed. Skip right after a cold snapshot (already
        // authoritative). Debounced, so cheap on rapid reconnects.
        if !lastPullStartedFromZero {
            Task { [weak self] in await self?.reconcile() }
        }
        // Stable-window timer: only reset reconnectAttempts after the
        // socket has been alive for 5s. Mirrors the TS engine's logic to
        // prevent a 1008-then-disconnect loop from clearing the backoff.
        stableTimer?.cancel()
        let stable = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 5_000_000_000)
            guard let self else { return }
            await self.markStable()
        }
        stableTimer = stable

        // Re-subscribe any active CRDT rows on the fresh socket.
        for key in crdtSubscriptions {
            let parts = key.split(separator: "\u{0000}", maxSplits: 1)
            if parts.count == 2 {
                let entity = String(parts[0])
                let rowId = String(parts[1])
                try? await socket.send(text: jsonEncodeMessage(["type": "crdt-subscribe", "entity": entity, "rowId": rowId]))
            }
        }

        receiveTask = Task { [weak self] in
            guard let self else { return }
            await self.consume(socket: socket)
        }
    }

    private func markStable() {
        reconnectAttempts = 0
        stableTimer = nil
    }

    private func consume(socket: PylonWebSocket) async {
        do {
            for try await message in socket.messages() {
                switch message {
                case .text(let text):
                    await handleTextFrame(text)
                case .binary(let data):
                    let handlers = binaryHandlers.values
                    for h in handlers {
                        h(data)
                    }
                }
            }
        } catch {
            // Stream errored — fall through to reconnect path.
        }
        wsConnected = false
        if running {
            scheduleReconnect()
        }
    }

    // MARK: - SSE transport (fallback)

    private func deriveSseURL() -> URL {
        if let override = config.sseURL { return override }
        var components = URLComponents(url: config.baseURL, resolvingAgainstBaseURL: false)!
        if let port = components.port {
            // pylon dev convention: SSE on port + 2 (WS on port + 1).
            components.port = port + 2
        }
        components.path = "/events"
        return components.url ?? config.baseURL
    }

    private func connectSse() async {
        guard running else { return }
        await refreshResolvedSession()
        let url = deriveSseURL()
        let token = await client.currentToken()
        let stream = SSEStream(url: url, token: token)
        sseStream = stream
        stream.connect()
        if !lastPullStartedFromZero {
            Task { [weak self] in await self?.reconcile() }
        }
        sseTask = Task { [weak self] in
            guard let self else { return }
            await self.consumeSse(stream: stream)
        }
    }

    private func consumeSse(stream: SSEStream) async {
        do {
            for try await event in stream.messages() {
                await handleTextFrame(event)
            }
        } catch {
            // Connection ended with an error — fall through to backoff.
        }
        if running {
            // Same exponential backoff as the WS path so SSE clients
            // don't form a second reconnect wave on server restart.
            reconnectAttempts += 1
            let delay = computeBackoff(attempts: reconnectAttempts, baseDelay: config.reconnectBaseDelay)
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                guard let self else { return }
                await self.pull()
                await self.connectSse()
            }
        }
    }

    // Internal (not private) so tests can inject a live frame mid-pull —
    // the leapfrog regression drives this exact entry point.
    func handleTextFrame(_ text: String) async {
        guard let data = text.data(using: .utf8),
              let parsed = try? JSONDecoder().decode(WSEnvelope.self, from: data) else {
            return
        }
        if let change = parsed.toChangeEvent() {
            // Pull fence: while a pull is in flight, hold live frames so
            // they can't advance the cursor past rows the pull hasn't
            // delivered yet (see `pullHold`). Replayed after the pull.
            if pullHold != nil {
                pullHold?.append(change)
                return
            }
            await applyLiveEvent(change)
        } else if parsed.type == "presence" {
            store.notify()
        }
    }

    /// Apply one live change event through the seq gate: already-seen
    /// seqs are dropped, newer ones apply and advance (+persist) the
    /// cursor. Shared by the direct WS path and the post-pull replay.
    private func applyLiveEvent(_ change: ChangeEvent) async {
        if change.seq > cursor.last_seq {
            await store.applyChangesAsync([change])
            cursor = SyncCursor(last_seq: change.seq)
            if let persistence {
                try? await persistence.saveCursor(cursor)
            }
        }
    }

    private func scheduleReconnect() {
        guard running else { return }
        reconnectAttempts += 1
        let delay = computeBackoff(attempts: reconnectAttempts, baseDelay: config.reconnectBaseDelay)
        Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self else { return }
            await self.pull()
            await self.connectWs()
        }
    }

    private func startPolling() {
        let interval = config.pollInterval
        pollTask = Task { [weak self] in
            while let self, await self.running {
                try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
                await self.push()
                await self.pull()
                // Periodic phantom-row sweep (debounced) so a delete/update
                // the poll's delta missed still converges.
                await self.reconcile()
            }
        }
    }

    // MARK: - Resolved session + identity flip

    public func currentResolvedSession() -> ResolvedSession {
        resolvedSession
    }

    public func refreshResolvedSession() async {
        do {
            let next = try await client.me()
            let tenantNow = next.tenantId
            if lastSeenTenantObserved && lastSeenTenant != tenantNow {
                if config.resetOnTenantFlip {
                    // Tenant flip is an identity change → wipe rows + queued writes.
                    await resetReplica(wipeMutations: true)
                } else {
                    // Membership-scoped reads: the replica is valid across all
                    // the user's orgs — keep every row, and reconcile to pick
                    // up rows of an org the user only just joined (their
                    // change events predate membership). Parity with the TS
                    // engine's resetOnTenantFlip: false path.
                    lastSeenTenant = tenantNow
                    lastSeenTenantObserved = true
                    if next != resolvedSession {
                        resolvedSession = next
                        store.notify()
                    }
                    await pull()
                    await reconcile()
                    return
                }
            }
            lastSeenTenant = tenantNow
            lastSeenTenantObserved = true
            if next != resolvedSession {
                resolvedSession = next
                store.notify()
            }
        } catch {
            // /api/auth/me failures are transient — let the next pull retry.
        }
    }

    public func notifySessionChanged() async {
        await refreshResolvedSession()
    }

    /// Drop the local replica and reset the cursor to 0 so the next pull
    /// resnapshots. Wipes BOTH memory and the on-disk rows (else a phantom
    /// replica rehydrates on next launch). `wipeMutations` is true on an
    /// IDENTITY flip (token/tenant) — the outgoing identity's un-pushed writes
    /// must not be replayed under the new token; false on a same-identity 410
    /// resnapshot, where pending offline writes are still valid and must
    /// survive. Mirrors the TS `resetReplicaInner({ wipeMutations })`.
    public func resetReplica(wipeMutations: Bool = false) async {
        cursor = SyncCursor()
        store.clearAll()
        if let persistence {
            try? await persistence.clearRows()
            try? await persistence.saveCursor(cursor)
        }
        if wipeMutations {
            await mutations.wipeAll()
        }
        // Replica wiped + cursor reset to 0 — the next pull re-snapshots from
        // scratch. Re-enter loading so an org switch shows a skeleton (not an
        // empty list) until that re-pull lands; re-arm the fallback so it can't
        // pin. The next pull's success re-settles it. (TS resetReplicaInner
        // parity — the useQuery loading-flash fix, framework #315.)
        _initialSyncSettled = false
        store.notify()
        armInitialSyncFallback()
    }

    // MARK: - Presence + topics

    public func setPresence(_ data: [String: JSONValue]) async {
        presenceData = data
        try? await sendWs(["type": "presence", "event": "update", "data": .object(data)])
    }

    public func publishTopic(_ topic: String, data: JSONValue) async {
        try? await sendWs(["type": "topic", "topic": .string(topic), "data": data])
    }

    private func sendWs(_ msg: [String: JSONValue]) async throws {
        guard let ws else { return }
        let text = jsonEncodeMessage(msg.mapValues { v -> Any in toAnyJSON(v) })
        try await ws.send(text: text)
    }

    private func sendWs(_ msg: [String: Any]) async throws {
        guard let ws else { return }
        let text = jsonEncodeMessage(msg)
        try await ws.send(text: text)
    }

    private func jsonEncodeMessage(_ msg: [String: Any]) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: msg) else { return "{}" }
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    private func toAnyJSON(_ v: JSONValue) -> Any {
        switch v {
        case .null: return NSNull()
        case .bool(let b): return b
        case .int(let i): return i
        case .double(let d): return d
        case .string(let s): return s
        case .array(let a): return a.map(toAnyJSON)
        case .object(let o): return o.mapValues(toAnyJSON)
        }
    }

    // MARK: - CRDT subscriptions

    public func subscribeCrdt(entity: String, rowId: String) async {
        let key = "\(entity)\u{0000}\(rowId)"
        let prev = crdtSubscribers[key] ?? 0
        crdtSubscribers[key] = prev + 1
        if prev == 0 {
            crdtSubscriptions.insert(key)
            try? await sendWs(["type": "crdt-subscribe", "entity": entity, "rowId": rowId])
        }
    }

    public func unsubscribeCrdt(entity: String, rowId: String) async {
        let key = "\(entity)\u{0000}\(rowId)"
        let prev = crdtSubscribers[key] ?? 0
        guard prev > 0 else { return }
        if prev == 1 {
            crdtSubscribers.removeValue(forKey: key)
            crdtSubscriptions.remove(key)
            try? await sendWs(["type": "crdt-unsubscribe", "entity": entity, "rowId": rowId])
        } else {
            crdtSubscribers[key] = prev - 1
        }
    }

    @discardableResult
    public func onBinaryFrame(_ handler: @escaping @Sendable (Data) -> Void) -> () -> Void {
        let id = UUID()
        binaryHandlers[id] = handler
        return { [weak self] in
            // Copy the weak binding into an immutable local before the
            // Task closure captures it — Swift 6 strict concurrency
            // rejects capturing the (mutable) weak `self` var directly
            // in concurrently-executing code.
            let engine = self
            Task { await engine?.removeBinaryHandler(id: id) }
        }
    }

    private func removeBinaryHandler(id: UUID) {
        binaryHandlers.removeValue(forKey: id)
    }

    // MARK: - Pagination

    /// Fetch one page from a typed entity. Mirrors the TS `loadPage` —
    /// uses cursor pagination (`/api/entities/{entity}/cursor?after=…`).
    public func loadPage<T: Decodable & Sendable>(
        _ entity: String,
        after: String? = nil,
        limit: Int = 20,
        as type: T.Type = T.self
    ) async throws -> CursorPage<T> {
        try await client.listCursor(entity, after: after, limit: limit)
    }

    /// Create an `InfiniteQuery` accumulator for an entity. Returns a value
    /// you can call `loadMore()` on; subscribers fire on each page-append.
    /// Marked `nonisolated` so SwiftUI wrappers can construct one inside
    /// `init` without an `await`.
    public nonisolated func createInfiniteQuery<T: Decodable & Sendable>(
        _ entity: String,
        pageSize: Int = 20,
        as type: T.Type = T.self
    ) -> InfiniteQuery<T> {
        InfiniteQuery(client: client, entity: entity, pageSize: pageSize)
    }

    // MARK: - Hydration

    /// Hydrate the local store from server-rendered data. Call before
    /// `start()` to skip a redundant initial pull.
    public func hydrate(_ data: HydrationData) {
        for (entity, rows) in data.entities {
            for row in rows {
                if let id = row["id"]?.stringValue {
                    store.applyChange(ChangeEvent(
                        seq: 0,
                        entity: entity,
                        row_id: id,
                        kind: .insert,
                        data: row
                    ))
                }
            }
        }
        if let c = data.cursor { cursor = c }
    }

    public func currentCursor() -> SyncCursor { cursor }
}

// MARK: - Wire envelope

/// Internal type used to peek at WebSocket frames before deciding whether
/// they're sync `ChangeEvent`s or control messages.
private struct WSEnvelope: Decodable {
    let seq: Int64?
    let entity: String?
    let row_id: String?
    let kind: ChangeKind?
    let data: [String: JSONValue]?
    let timestamp: String?
    let type: String?

    func toChangeEvent() -> ChangeEvent? {
        guard let seq, let entity, let row_id, let kind else { return nil }
        return ChangeEvent(
            seq: seq,
            entity: entity,
            row_id: row_id,
            kind: kind,
            data: data,
            timestamp: timestamp ?? ""
        )
    }
}

// MARK: - Persistence protocol

/// Persistence backend for the sync engine — entity rows, sync cursor, and
/// the mutation queue. The `SQLiteSyncPersistence` impl in this module
/// satisfies it. Apps targeting environments without filesystem access
/// (Workers, in-memory tests) can pass `nil` to skip persistence.
public protocol SyncPersistence: MutationQueuePersistence {
    func loadAllRows() async throws -> [String: [Row]]
    func loadCursor() async throws -> SyncCursor?
    func saveCursor(_ cursor: SyncCursor) async throws
    func persist(_ change: ChangeEvent) async throws
    /// Delete every persisted entity row. Used by `resetReplica` so an identity
    /// flip / 410 resnapshot doesn't leave the previous identity's (or stale)
    /// rows on disk to be rehydrated on next launch — the on-disk half of the
    /// "cross-identity read leak". Does NOT touch the cursor or the mutation
    /// queue (the engine resets those explicitly).
    func clearRows() async throws
}
