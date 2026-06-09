import XCTest
import PylonClient
@testable import PylonSync

/// Test transport copy. PylonClientTests' `MockTransport` lives in a
/// different test bundle and can't be imported here.
final class MockTransport: PylonHTTPTransport, @unchecked Sendable {
    typealias Handler = @Sendable (URLRequest) async throws -> (Int, Data)
    private let lock = NSLock()
    private var handler: Handler

    init(_ handler: @escaping Handler = { _ in (200, Data()) }) { self.handler = handler }

    func setHandler(_ h: @escaping Handler) {
        lock.lock(); defer { lock.unlock() }
        handler = h
    }

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let h: Handler = { lock.lock(); defer { lock.unlock() }; return handler }()
        let (status, data) = try await h(request)
        return (data, HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json"])!)
    }

    func upload(_ request: URLRequest, body: Data) async throws -> (Data, HTTPURLResponse) {
        try await send(request)
    }
}

/// End-to-end test against a fake HTTP transport that mimics the pylon
/// server. Exercises pull → optimistic insert → push → cursor advance.
final class SyncEngineIntegrationTests: XCTestCase {

    actor FakeServer {
        var rows: [String: [String: Row]] = [:]
        var seq: Int64 = 0
        var changes: [ChangeEvent] = []

        func handle(_ req: URLRequest) throws -> (Int, Data) {
            let path = req.url?.path ?? ""
            let method = req.httpMethod ?? "GET"
            switch (method, path) {
            case ("GET", "/api/auth/me"):
                let body: [String: Any] = [
                    "user_id": "u1", "tenant_id": NSNull(), "is_admin": false, "roles": []
                ]
                return (200, try JSONSerialization.data(withJSONObject: body))

            case ("GET", "/api/sync/pull"):
                let resp: [String: Any] = [
                    "changes": changes.map { c -> [String: Any] in
                        var d: [String: Any] = [
                            "seq": c.seq,
                            "entity": c.entity,
                            "row_id": c.row_id,
                            "kind": c.kind.rawValue,
                            "timestamp": c.timestamp,
                        ]
                        if let data = c.data {
                            d["data"] = jsonValueToAny(.object(data))
                        }
                        return d
                    },
                    "cursor": ["last_seq": seq],
                    "has_more": false
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))

            case ("POST", "/api/sync/push"):
                guard let body = req.httpBody,
                      let parsed = try? JSONDecoder().decode(PushRequest.self, from: body) else {
                    return (400, Data("{}".utf8))
                }
                var applied = 0
                for change in parsed.changes {
                    seq += 1
                    let event = ChangeEvent(seq: seq, entity: change.entity, row_id: change.row_id, kind: change.kind, data: change.data ?? [:], timestamp: "")
                    changes.append(event)
                    if rows[change.entity] == nil { rows[change.entity] = [:] }
                    if change.kind == .delete {
                        rows[change.entity]?.removeValue(forKey: change.row_id)
                    } else {
                        rows[change.entity]?[change.row_id] = change.data ?? [:]
                    }
                    applied += 1
                }
                let resp: [String: Any] = [
                    "applied": applied,
                    "errors": [],
                    "cursor": ["last_seq": seq]
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))

            default:
                return (404, Data("{}".utf8))
            }
        }
    }

    func testInsertGoesThroughPushAndUpdatesServerView() async throws {
        let server = FakeServer()
        let transport = MockTransport()
        transport.setHandler { [server] req in
            try await server.handle(req)
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        await client.setSession(token: "tok")
        let cfg = SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client)
        // Don't start() — that would launch the WS transport which would
        // race the test. Just exercise pull/push directly.
        await engine.refreshResolvedSession()
        await engine.pull()
        let beforeStore = await engine.store
        XCTAssertEqual(beforeStore.list("Todo").count, 0)

        let _ = await engine.insert("Todo", ["title": "ship the swift sdk"])
        // After push, the server should hold the row.
        let serverRows = await server.rows
        XCTAssertEqual(serverRows["Todo"]?.count, 1)
    }

    func testPullAdvancesCursor() async throws {
        let server = FakeServer()
        // Seed the server with one change so the next pull has data.
        await server.seedChange(ChangeEvent(seq: 5, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "preexisting"]))

        let transport = MockTransport()
        transport.setHandler { [server] req in try await server.handle(req) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        let engine = await SyncEngine(config: SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll), client: client)
        await engine.pull()
        let cursor = await engine.currentCursor()
        XCTAssertEqual(cursor.last_seq, 5)
        let store = await engine.store
        XCTAssertEqual(store.get("Todo", id: "t1")?["title"]?.stringValue, "preexisting")
    }
}

extension SyncEngineIntegrationTests.FakeServer {
    func seedChange(_ change: ChangeEvent) {
        seq = max(seq, change.seq)
        changes.append(change)
        if rows[change.entity] == nil { rows[change.entity] = [:] }
        if change.kind != .delete {
            rows[change.entity]?[change.row_id] = change.data ?? [:]
        }
    }

    // P0-2: resetReplica(wipeMutations:) must wipe the ON-DISK rows, not just
    // memory — else user A's SQLite rows rehydrate into user B's session on the
    // next launch (the cross-identity read leak on the Mac app).
    func testResetReplicaWipesOnDiskRows() async throws {
        let path = NSTemporaryDirectory() + "pylon_reset_\(UUID().uuidString).db"
        defer { try? FileManager.default.removeItem(atPath: path) }
        let persistence = try SQLitePersistence(path: path)
        // Seed the previous identity's rows on disk.
        try await persistence.persist(ChangeEvent(seq: 1, entity: "Recording", row_id: "r1", kind: .insert, data: ["t": "secret"]))
        let seeded = try await persistence.loadAllRows()
        XCTAssertFalse(seeded.isEmpty)

        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: MockTransport()
        )
        let engine = await SyncEngine(
            config: SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60),
            client: client,
            persistence: persistence
        )
        await engine.resetReplica(wipeMutations: true)

        let rowsAfter = try await persistence.loadAllRows()
        XCTAssertTrue(rowsAfter.isEmpty, "resetReplica must wipe disk rows, not just memory")
        let store = await engine.store
        XCTAssertNil(store.get("Recording", id: "r1"), "memory replica also cleared")
        let cursorAfter = await engine.currentCursor()
        XCTAssertEqual(cursorAfter.last_seq, 0, "cursor reset to 0")
    }
}

private func jsonValueToAny(_ v: JSONValue) -> Any {
    switch v {
    case .null: return NSNull()
    case .bool(let b): return b
    case .int(let i): return i
    case .double(let d): return d
    case .string(let s): return s
    case .array(let a): return a.map(jsonValueToAny)
    case .object(let o): return o.mapValues(jsonValueToAny)
    }
}
