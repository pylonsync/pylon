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
        appName: String = "default"
    ) {
        self.baseURL = baseURL
        self.wsURL = wsURL
        self.sseURL = sseURL
        self.transport = transport
        self.pollInterval = pollInterval
        self.reconnectBaseDelay = reconnectBaseDelay
        self.appName = appName
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
        await pull()
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
            await resetReplica()
            Task { await self.refreshResolvedSession() }
        }
        lastSeenToken = tokenNow
        lastSeenTokenObserved = true

        do {
            let resp = try await client.syncPull(since: cursor.last_seq)
            consecutive410s = 0
            if !resp.changes.isEmpty {
                await store.applyChangesAsync(resp.changes)
            }
            if resp.cursor.last_seq > cursor.last_seq {
                cursor = resp.cursor
                if let persistence {
                    try? await persistence.saveCursor(cursor)
                }
            }
            if resp.has_more {
                await pull()
            }
        } catch let error as PylonError {
            switch error.httpStatus {
            case 429:
                reconnectAttempts += 3
            case 410:
                let attempt = consecutive410s
                consecutive410s += 1
                if attempt == 0 {
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
            // Other transport errors swallow — caller will retry on next tick.
        }
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
            for (i, m) in pending.enumerated() {
                if i < resp.applied {
                    await mutations.markApplied(m.id)
                } else {
                    let idx = i - resp.applied
                    if idx < resp.errors.count {
                        await mutations.markFailed(m.id, error: resp.errors[idx])
                    }
                }
            }
            await mutations.clear()
        } catch {
            // Server retries are idempotent via op_id — leave pending and try again.
        }
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
        store.optimisticUpdate(entity, id: id, data)
        await mutations.add(ClientChange(entity: entity, row_id: id, kind: .update, data: data))
        await push()
    }

    public func delete(_ entity: String, id: String) async {
        store.optimisticDelete(entity, id: id)
        await mutations.add(ClientChange(entity: entity, row_id: id, kind: .delete))
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

    private func connectWs() async {
        guard running else { return }
        let url = deriveWsURL()
        let token = await client.currentToken()
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
        let url = deriveSseURL()
        let token = await client.currentToken()
        let stream = SSEStream(url: url, token: token)
        sseStream = stream
        stream.connect()
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

    private func handleTextFrame(_ text: String) async {
        guard let data = text.data(using: .utf8),
              let parsed = try? JSONDecoder().decode(WSEnvelope.self, from: data) else {
            return
        }
        if let change = parsed.toChangeEvent() {
            if change.seq > cursor.last_seq {
                await store.applyChangesAsync([change])
                cursor = SyncCursor(last_seq: change.seq)
                if let persistence {
                    try? await persistence.saveCursor(cursor)
                }
            }
        } else if parsed.type == "presence" {
            store.notify()
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
                await resetReplica()
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

    public func resetReplica() async {
        cursor = SyncCursor()
        store.clearAll()
        if let persistence {
            try? await persistence.saveCursor(cursor)
        }
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
            Task { await self?.removeBinaryHandler(id: id) }
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
}
