import XCTest
import PylonClient
@testable import PylonSync

/// Sign-in without a restart must re-sync EVERY entity. Parity with
/// `packages/sync/src/identity-resync.test.ts`.
///
/// The session refresh after sign-in saw a user flip with no tenant flip
/// and only updated the cached session: no wipe, no pull, and the socket
/// stayed bound to the anonymous identity. Concurrent scoped reconciles
/// also dropped every request after the first.
final class IdentityResyncTests: XCTestCase {

    /// Rows are visible only to a signed-in caller. The bearer token maps
    /// to a user id; no token means anonymous.
    actor Server {
        var changes: [ChangeEvent] = []
        var seq: Int64 = 0
        var users: [String: String] = [:] // token → user id
        var owners: [String: String] = [:] // row id → owner (nil = any signed-in user)
        var pullSinces: [Int64] = []

        func addUser(token: String, userId: String) { users[token] = userId }
        func seed(_ entity: String, _ id: String, owner: String? = nil) {
            seq += 1
            changes.append(ChangeEvent(seq: seq, entity: entity, row_id: id, kind: .insert,
                                       data: ["id": .string(id)], timestamp: ""))
            if let owner { owners[id] = owner }
        }
        func rows(_ entity: String, visibleTo user: String?) -> [ChangeEvent] {
            guard let user else { return [] }
            return changes.filter { $0.entity == entity && (owners[$0.row_id] ?? user) == user }
        }

        func handle(_ req: URLRequest) throws -> (Int, Data) {
            let path = req.url?.path ?? ""
            let auth = req.value(forHTTPHeaderField: "Authorization") ?? ""
            let token = auth.hasPrefix("Bearer ") ? String(auth.dropFirst(7)) : nil
            let user = token.flatMap { users[$0] }
            switch path {
            case "/api/auth/me":
                let body: [String: Any] = [
                    "user_id": user ?? NSNull(), "tenant_id": NSNull(), "is_admin": false, "roles": [],
                ]
                return (200, try JSONSerialization.data(withJSONObject: body))
            case "/api/sync/pull":
                let comps = URLComponents(url: req.url!, resolvingAgainstBaseURL: false)
                let since = Int64(comps?.queryItems?.first(where: { $0.name == "since" })?.value ?? "0") ?? 0
                pullSinces.append(since)
                let visible = changes.filter {
                    $0.seq > since && user != nil && (owners[$0.row_id] ?? user!) == user!
                }
                let resp: [String: Any] = [
                    "changes": visible.map { c -> [String: Any] in
                        ["seq": c.seq, "entity": c.entity, "row_id": c.row_id,
                         "kind": c.kind.rawValue, "timestamp": "", "data": ["id": c.row_id]]
                    },
                    "cursor": ["last_seq": seq],
                    "has_more": false,
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            default:
                if path.hasPrefix("/api/entities/"), path.hasSuffix("/cursor") {
                    let entity = String(path.dropFirst("/api/entities/".count).dropLast("/cursor".count))
                    let data = rows(entity, visibleTo: user).map { ["id": $0.row_id] }
                    let resp: [String: Any] = ["data": data, "has_more": false, "next_cursor": NSNull()]
                    return (200, try JSONSerialization.data(withJSONObject: resp))
                }
                return (404, Data("{}".utf8))
            }
        }
    }

    final class SocketLog: @unchecked Sendable {
        private let lock = NSLock()
        private(set) var tokens: [String?] = []
        func record(_ token: String?) { lock.lock(); tokens.append(token); lock.unlock() }
        var count: Int { lock.lock(); defer { lock.unlock() }; return tokens.count }
    }

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
        webSocketFactory: (@Sendable (URL, String?) -> PylonWebSocket)? = nil
    ) async -> (SyncEngine, PylonClient) {
        let mock = MockTransport()
        mock.setHandler { [server] req in try await server.handle(req) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: mock
        )
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!,
            transport: transport,
            pollInterval: 60,
            reconnectBaseDelay: 0.01
        )
        let engine = await SyncEngine(config: cfg, client: client, webSocketFactory: webSocketFactory)
        return (engine, client)
    }

    func testAnonymousToSignedInPullsEveryEntity() async throws {
        let server = Server()
        await server.addUser(token: "tokU1", userId: "u1")
        await server.seed("Company", "c1")
        await server.seed("Company", "c2")
        await server.seed("Deal", "d1")
        await server.seed("Deal", "d2")
        await server.seed("Deal", "d3")
        let (engine, client) = await makeEngine(server)
        await engine.refreshResolvedSession()
        await engine.pull()
        let store = await engine.store
        XCTAssertEqual(store.list("Company").count, 0)

        // Sign in: store the token, then refresh the session.
        await client.setSession(token: "tokU1")
        await engine.notifySessionChanged()

        let userId = await engine.currentResolvedSession().userId
        XCTAssertEqual(userId, "u1")
        XCTAssertEqual(store.list("Company").count, 2)
        XCTAssertEqual(store.list("Deal").count, 3)
        let sinces = await server.pullSinces
        XCTAssertEqual(sinces.last, 0, "the sign-in re-pulls from zero")
    }

    func testUserAToUserBWipesAsRows() async throws {
        let server = Server()
        await server.addUser(token: "tokA", userId: "uA")
        await server.addUser(token: "tokB", userId: "uB")
        await server.seed("Note", "a1", owner: "uA")
        await server.seed("Note", "b1", owner: "uB")
        let (engine, client) = await makeEngine(server)
        await client.setSession(token: "tokA")
        await engine.refreshResolvedSession()
        await engine.pull()
        let store = await engine.store
        XCTAssertEqual(store.list("Note").compactMap { $0["id"]?.stringValue }, ["a1"])

        await client.setSession(token: "tokB")
        await engine.notifySessionChanged()
        XCTAssertEqual(store.list("Note").compactMap { $0["id"]?.stringValue }, ["b1"])
    }

    func testSignInReconnectsTheSocketWithTheNewToken() async throws {
        let server = Server()
        await server.addUser(token: "tokU1", userId: "u1")
        let log = SocketLog()
        let (engine, client) = await makeEngine(server, transport: .websocket) { _, token in
            log.record(token)
            return IdleSocket()
        }
        await engine.start()
        XCTAssertEqual(log.count, 1)
        XCTAssertEqual(log.tokens.first ?? nil, nil)

        await client.setSession(token: "tokU1")
        await engine.notifySessionChanged()
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertEqual(log.count, 2, "one new socket for the signed-in identity")
        XCTAssertEqual(log.tokens.last ?? nil, "tokU1")
        await engine.stop()
    }

    /// The flip cycles the socket, and the reconnect's `connectWs` refreshes
    /// the session while the flip's own pull is still running. It must not
    /// see a second flip and reset + re-snapshot again.
    func testSignInOnTheWebSocketTransportResetsAndSnapshotsOnce() async throws {
        let server = Server()
        await server.addUser(token: "tokU1", userId: "u1")
        await server.seed("Company", "c1")
        let (engine, client) = await makeEngine(server, transport: .websocket) { _, _ in IdleSocket() }
        await engine.start()
        let before = await server.pullSinces.count

        await client.setSession(token: "tokU1")
        await engine.notifySessionChanged()
        try await Task.sleep(nanoseconds: 300_000_000)
        let after = Array(await server.pullSinces.dropFirst(before))
        XCTAssertEqual(after.filter { $0 == 0 }.count, 1, "one from-zero snapshot per sign-in, saw \(after)")
        let store = await engine.store
        XCTAssertEqual(store.list("Company").count, 1)
        await engine.stop()
    }

    func testConcurrentScopedReconcilesFetchEveryEntity() async throws {
        let server = Server()
        await server.addUser(token: "tokU1", userId: "u1")
        let (engine, client) = await makeEngine(server)
        await client.setSession(token: "tokU1")
        await engine.refreshResolvedSession()
        await engine.pull()
        await server.seed("Company", "c1")
        await server.seed("Deal", "d1")
        await server.seed("Deal", "d2")

        async let a: Void = engine.reconcile(["Company"])
        async let b: Void = engine.reconcile(["Deal"])
        _ = await (a, b)

        let store = await engine.store
        XCTAssertEqual(store.list("Company").count, 1)
        XCTAssertEqual(store.list("Deal").count, 2)
    }
}
