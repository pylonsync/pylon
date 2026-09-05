import XCTest
import PylonClient
@testable import PylonSync

/// Parity tests for the behaviors the 2026-06 sync review found missing
/// from the Swift engine (P1-1 … P1-7). Each mirrors a scenario the TS
/// engine already pins in `packages/sync/src/scenarios.test.ts`.
final class SyncEngineParityP1Tests: XCTestCase {

    /// A scriptable fake pylon server: rows, a change log, a mutable
    /// `/api/auth/me` body, a queue of push outcomes, and a request log.
    actor Server {
        var rows: [String: [String: Row]] = [:]
        var seq: Int64 = 0
        var changes: [ChangeEvent] = []
        var me: [String: Any] = ["user_id": "u1", "tenant_id": NSNull(), "is_admin": false, "roles": []]
        /// HTTP statuses to answer the next pushes with (200 = apply).
        var pushOutcomes: [Int] = []
        var requests: [(method: String, path: String, query: String?)] = []
        var pullSinces: [Int64] = []

        func setMe(userId: String, tenantId: String?) {
            me = ["user_id": userId, "tenant_id": tenantId ?? NSNull(), "is_admin": false, "roles": []]
        }
        func primePush(_ statuses: [Int]) { pushOutcomes = statuses }
        func seed(_ change: ChangeEvent) {
            seq = max(seq, change.seq)
            changes.append(change)
            if rows[change.entity] == nil { rows[change.entity] = [:] }
            if change.kind == .delete {
                rows[change.entity]?.removeValue(forKey: change.row_id)
            } else {
                var row = change.data ?? [:]
                row["id"] = .string(change.row_id)
                rows[change.entity]?[change.row_id] = row
            }
        }
        func paths(startingWith prefix: String) -> [String] {
            requests.map(\.path).filter { $0.hasPrefix(prefix) }
        }
        func meCount() -> Int { requests.filter { $0.path == "/api/auth/me" }.count }
        func pushCount() -> Int { requests.filter { $0.path == "/api/sync/push" }.count }

        func handle(_ req: URLRequest) throws -> (Int, Data) {
            let path = req.url?.path ?? ""
            let method = req.httpMethod ?? "GET"
            requests.append((method, path, req.url?.query))
            switch (method, path) {
            case ("GET", "/api/auth/me"):
                return (200, try JSONSerialization.data(withJSONObject: me))
            case ("GET", "/api/sync/pull"):
                let comps = URLComponents(url: req.url!, resolvingAgainstBaseURL: false)
                let since = Int64(comps?.queryItems?.first(where: { $0.name == "since" })?.value ?? "0") ?? 0
                pullSinces.append(since)
                let visible = changes.filter { $0.seq > since }
                let resp: [String: Any] = [
                    "changes": visible.map { c -> [String: Any] in
                        var d: [String: Any] = [
                            "seq": c.seq, "entity": c.entity, "row_id": c.row_id,
                            "kind": c.kind.rawValue, "timestamp": c.timestamp,
                        ]
                        if let data = c.data { d["data"] = anyJSON(.object(data)) }
                        return d
                    },
                    "cursor": ["last_seq": seq],
                    "has_more": false,
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            case ("POST", "/api/sync/push"):
                if !pushOutcomes.isEmpty {
                    let status = pushOutcomes.removeFirst()
                    if status != 200 {
                        return (status, Data(#"{"error":{"code":"UNAVAILABLE","message":"try later"}}"#.utf8))
                    }
                }
                let body = req.httpBody ?? Data()
                let push = try JSONDecoder().decode(PushRequest.self, from: body)
                var results: [[String: Any]] = []
                for change in push.changes {
                    seq += 1
                    if rows[change.entity] == nil { rows[change.entity] = [:] }
                    switch change.kind {
                    case .delete:
                        rows[change.entity]?.removeValue(forKey: change.row_id)
                    default:
                        var row = change.data ?? [:]
                        row["id"] = .string(change.row_id)
                        rows[change.entity]?[change.row_id] = row
                    }
                    changes.append(ChangeEvent(seq: seq, entity: change.entity, row_id: change.row_id,
                                               kind: change.kind, data: change.data, timestamp: ""))
                    results.append(["op_id": change.op_id ?? "", "status": "applied", "seq": seq])
                }
                let resp: [String: Any] = [
                    "applied": push.changes.count, "errors": [], "cursor": ["last_seq": seq],
                    "results": results, "max_applied_seq": seq,
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            default:
                // Reconcile fetch: GET /api/entities/<E>/cursor → the
                // entity's current rows, one page.
                if method == "GET", path.hasPrefix("/api/entities/"), path.hasSuffix("/cursor") {
                    let entity = String(path.dropFirst("/api/entities/".count).dropLast("/cursor".count))
                    let data = (rows[entity] ?? [:]).values.map { anyJSON(.object($0)) }
                    let resp: [String: Any] = ["data": data, "has_more": false, "next_cursor": NSNull()]
                    return (200, try JSONSerialization.data(withJSONObject: resp))
                }
                return (404, Data("{}".utf8))
            }
        }
    }

    /// Counts connects and records the token each socket was opened with.
    final class SocketLog: @unchecked Sendable {
        private let lock = NSLock()
        private(set) var tokens: [String?] = []
        func record(_ token: String?) { lock.lock(); tokens.append(token); lock.unlock() }
        var count: Int { lock.lock(); defer { lock.unlock() }; return tokens.count }
    }

    /// A socket that stays open until closed and delivers nothing.
    final class IdleSocket: PylonWebSocket, @unchecked Sendable {
        private var continuation: AsyncThrowingStream<WSMessage, Error>.Continuation?
        func connect() async throws {}
        func send(text: String) async throws {}
        func send(binary: Data) async throws {}
        func messages() -> AsyncThrowingStream<WSMessage, Error> {
            AsyncThrowingStream { c in self.continuation = c }
        }
        func close() { continuation?.finish() }
    }

    private func makeEngine(
        _ server: Server,
        transport: SyncEngineConfig.TransportType = .poll,
        pollInterval: TimeInterval = 60,
        resetOnTenantFlip: Bool = true,
        token: String = "tok1",
        webSocketFactory: (@Sendable (URL, String?) -> PylonWebSocket)? = nil
    ) async -> (SyncEngine, PylonClient) {
        let transportMock = MockTransport()
        transportMock.setHandler { [server] req in try await server.handle(req) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transportMock
        )
        await client.setSession(token: token)
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!,
            transport: transport,
            pollInterval: pollInterval,
            reconnectBaseDelay: 0.01,
            resetOnTenantFlip: resetOnTenantFlip
        )
        let engine = await SyncEngine(config: cfg, client: client, webSocketFactory: webSocketFactory)
        return (engine, client)
    }

    // MARK: P1-7 — the ghost and the canonical row share one id

    func testInsertGhostAndServerRowShareId() async throws {
        let server = Server()
        let (engine, _) = await makeEngine(server)
        await engine.pull()
        let id = await engine.insert("Todo", ["title": "ship"])
        XCTAssertEqual(id.count, 40, "Pylon-shaped id: 32 hex nanos + 8 hex counter")
        XCTAssertTrue(id.allSatisfy { $0.isHexDigit })
        let store = await engine.store
        XCTAssertEqual(store.list("Todo").count, 1, "one row, not a ghost plus a canonical copy")
        XCTAssertEqual(store.get("Todo", id: id)?["title"]?.stringValue, "ship")
        let serverRows = await server.rows
        XCTAssertNotNil(serverRows["Todo"]?[id], "the server stored the row under the client id")
        // The server's own change event for the row lands on the same id.
        await engine.pull()
        XCTAssertEqual(store.list("Todo").count, 1)
    }

    // MARK: P1-2 — an optimistic delete fence is released by the server's delete

    func testOptimisticDeleteFenceIsReleasedByTheServerDelete() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "a"]))
        store.optimisticDelete("Todo", id: "t1")
        XCTAssertNil(store.get("Todo", id: "t1"))
        // Fenced: a late update for the row must not resurrect it.
        store.applyChange(ChangeEvent(seq: 6, entity: "Todo", row_id: "t1", kind: .update, data: ["title": "late"]))
        XCTAssertNil(store.get("Todo", id: "t1"))
        // The server's delete confirms it and lifts the fence …
        store.applyChange(ChangeEvent(seq: 7, entity: "Todo", row_id: "t1", kind: .delete, data: nil))
        // … so a later legitimate re-create of the id lands. With the old
        // Int64.max tombstone this row could never come back.
        store.applyChange(ChangeEvent(seq: 8, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "again"]))
        XCTAssertEqual(store.get("Todo", id: "t1")?["title"]?.stringValue, "again")
    }

    func testRestoreRowYieldsToAServerTombstone() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "a"]))
        store.optimisticDelete("Todo", id: "t1")
        // The server deleted the row while the (rejected) push was in flight.
        store.applyChange(ChangeEvent(seq: 6, entity: "Todo", row_id: "t1", kind: .delete, data: nil))
        store.restoreRow("Todo", id: "t1", prev: ["id": "t1", "title": "a"])
        XCTAssertNil(store.get("Todo", id: "t1"), "the server's delete outranks the local rollback")
    }

    // MARK: P1-3 — a transient push failure retries with backoff

    func testTransientPushFailureRetries() async throws {
        let server = Server()
        await server.primePush([503])
        let (engine, _) = await makeEngine(server)
        await engine.pull()
        let id = await engine.insert("Todo", ["title": "offline"])
        // First push failed (503) and nothing else drives a retry in
        // WS/SSE mode — the engine's own backoff must.
        let v1 = await server.pushCount()
        XCTAssertEqual(v1, 1)
        let v2 = await server.rows["Todo"]?.count ?? 0
        XCTAssertEqual(v2, 0)
        let v3 = await engine.mutations.pending().count
        XCTAssertEqual(v3, 1, "still queued, ghost kept")
        try await Task.sleep(nanoseconds: 1_500_000_000)
        let v4 = await server.pushCount()
        XCTAssertGreaterThanOrEqual(v4, 2, "retried after ~1s")
        let v5 = await server.rows["Todo"]?[id]
        XCTAssertNotNil(v5)
        let v6 = await engine.mutations.pending().count
        XCTAssertEqual(v6, 0)
    }

    // MARK: P1-4 — row-revoked and session-changed envelopes

    func testRowRevokedFrameRemovesTheRowAndBlocksStaleReplay() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "visible"]))
        let (engine, _) = await makeEngine(server)
        await engine.pull()
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: "n1"))

        await engine.handleTextFrame(#"{"type":"row-revoked","entity":"Note","row_id":"n1","seq":9}"#)
        XCTAssertNil(store.get("Note", id: "n1"))
        // A stale insert below the revocation seq must not resurrect it.
        await engine.handleTextFrame(#"{"type":"change","seq":1,"entity":"Note","row_id":"n1","kind":"insert","data":{"id":"n1","title":"STALE"}}"#)
        XCTAssertNil(store.get("Note", id: "n1"))
    }

    func testSessionChangedFrameRefreshesTheSession() async throws {
        let server = Server()
        let (engine, _) = await makeEngine(server)
        await engine.refreshResolvedSession()
        let before = await server.meCount()
        await engine.handleTextFrame(#"{"type":"session-changed"}"#)
        let v7 = await server.meCount()
        XCTAssertEqual(v7, before + 1, "the envelope re-reads /api/auth/me")
    }

    // MARK: P1-5 — tenant flip verdicts

    func testFirstTenantResolutionKeepsTheReplicaAndPulls() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "keep"]))
        let (engine, _) = await makeEngine(server)
        await engine.refreshResolvedSession() // tenant: null observed
        await engine.pull()
        let sincesBefore = await server.pullSinces

        // null → X: the engine started before select-org landed. The
        // cached rows ARE for this tenant, so no wipe, just a pull.
        await server.setMe(userId: "u1", tenantId: "org-a")
        await engine.refreshResolvedSession()
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: "n1"), "first resolution must not wipe")
        let sinces = await server.pullSinces
        XCTAssertEqual(sinces.count, sincesBefore.count + 1, "a pull ran after the flip")
        XCTAssertEqual(sinces.last, 5, "delta pull from the kept cursor, not a from-zero snapshot")
        let v8 = await engine.currentResolvedSession().tenantId
        XCTAssertEqual(v8, "org-a")
    }

    func testTenantFlipWipesTheReplicaAndRepullsFromZero() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "org-a row"]))
        await server.setMe(userId: "u1", tenantId: "org-a")
        let (engine, _) = await makeEngine(server)
        await engine.refreshResolvedSession()
        await engine.pull()
        let v9 = await engine.currentCursor().last_seq
        XCTAssertEqual(v9, 5)
        // Queue an offline write under org-a: it must not survive the flip.
        await server.primePush([503])
        _ = await engine.insert("Note", ["title": "draft"])
        let v10 = await engine.mutations.pending().count
        XCTAssertEqual(v10, 1)

        await server.setMe(userId: "u1", tenantId: "org-b")
        await engine.refreshResolvedSession()
        let sinces = await server.pullSinces
        XCTAssertEqual(sinces.last, 0, "X → Y wipes and re-pulls from zero")
        let v11 = await engine.mutations.pending().count
        XCTAssertEqual(v11, 0, "the outgoing tenant's queued writes are dropped")
        let v12 = await engine.currentResolvedSession().tenantId
        XCTAssertEqual(v12, "org-b")
        // Let the retry timer from the primed 503 fire harmlessly.
        try await Task.sleep(nanoseconds: 1_200_000_000)
    }

    func testTenantFlipWithResetDisabledKeepsRowsAndReconciles() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "shared"]))
        await server.setMe(userId: "u1", tenantId: "org-a")
        let (engine, _) = await makeEngine(server, resetOnTenantFlip: false)
        await engine.refreshResolvedSession()
        await engine.pull()

        await server.setMe(userId: "u1", tenantId: "org-b")
        await engine.refreshResolvedSession()
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: "n1"), "membership-scoped: rows survive the org switch")
        let v13 = await server.pullSinces.last
        XCTAssertEqual(v13, 5, "delta pull, no snapshot")
        let v14 = await server.paths(startingWith: "/api/entities/").isEmpty
        XCTAssertFalse(v14,
                       "a reconcile sweep picks up rows of the org the user just joined")
    }

    // MARK: P1-6 — the poll tick does not reconcile

    func testPollTickPushesAndPullsWithoutReconciling() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "a"]))
        let (engine, _) = await makeEngine(server, transport: .poll, pollInterval: 0.05)
        await engine.start()
        try await Task.sleep(nanoseconds: 400_000_000)
        await engine.stop()
        let pulls = await server.pullSinces.count
        XCTAssertGreaterThanOrEqual(pulls, 3, "the tick keeps pulling")
        let v15 = await server.paths(startingWith: "/api/entities/").isEmpty
        XCTAssertTrue(v15,
                      "no per-entity refetch on every tick (that re-downloaded every table)")
    }

    // MARK: F4 parity — an identity flip cycles the live socket

    func testIdentityFlipReconnectsTheWebSocketWithTheNewToken() async throws {
        let server = Server()
        let log = SocketLog()
        let (engine, client) = await makeEngine(server, transport: .websocket) { _, token in
            log.record(token)
            return IdleSocket()
        }
        await engine.start()
        XCTAssertEqual(log.count, 1)
        XCTAssertEqual(log.tokens.first ?? nil, "tok1")

        await client.setSession(token: "tok2")
        await engine.pull() // the token-flip path
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertEqual(log.count, 2, "exactly one new socket: the cycled connect, no reconnect storm")
        XCTAssertEqual(log.tokens.last ?? nil, "tok2", "the new socket binds the new identity")
        await engine.stop()
    }

    // MARK: P1-1 — pull apply is seq-monotonic

    func testPullDoesNotReapplyAnAlreadySeenSeq() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "server"]))
        let (engine, _) = await makeEngine(server)
        await engine.pull()
        let store = await engine.store
        // A local edit newer than seq 5 that the server has not echoed yet.
        store.optimisticUpdate("Note", id: "n1", ["title": "local-newer"])
        // A retransmit of seq 5 (WS + pull window overlap) must not clobber it.
        await engine.handleTextFrame(#"{"type":"change","seq":5,"entity":"Note","row_id":"n1","kind":"insert","data":{"id":"n1","title":"server"}}"#)
        XCTAssertEqual(store.get("Note", id: "n1")?["title"]?.stringValue, "local-newer")
    }
}

private func anyJSON(_ v: JSONValue) -> Any {
    switch v {
    case .null: return NSNull()
    case .bool(let b): return b
    case .int(let i): return i
    case .double(let d): return d
    case .string(let s): return s
    case .array(let a): return a.map(anyJSON)
    case .object(let o): return o.mapValues(anyJSON)
    }
}
