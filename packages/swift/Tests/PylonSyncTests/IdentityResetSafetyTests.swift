import XCTest
import PylonClient
@testable import PylonSync

/// Identity changes must not reset twice, let the old socket's frames into
/// the new replica, or throw away a signed-out user's offline writes.
/// Parity with `packages/sync/src/identity-reset-safety.test.ts`.
final class IdentityResetSafetyTests: XCTestCase {
    typealias Server = SyncEngineParityP1Tests.Server

    /// SQLite persistence whose `clearRows` waits for `release()`, so a
    /// test can deliver a frame while a reset is running.
    final class GatedPersistence: SyncPersistence, @unchecked Sendable {
        let inner: SQLitePersistence
        private let lock = NSLock()
        private var gate: CheckedContinuation<Void, Never>?
        private var armed = false
        private(set) var clearStarted = false

        init(path: String) throws { inner = try SQLitePersistence(path: path) }
        private func locked<T>(_ body: () -> T) -> T { lock.lock(); defer { lock.unlock() }; return body() }
        func arm() { locked { armed = true } }
        func started() -> Bool { locked { clearStarted } }
        func release() {
            let c: CheckedContinuation<Void, Never>? = locked { let g = gate; gate = nil; armed = false; return g }
            c?.resume()
        }
        func clearRows() async throws {
            let wait = locked { () -> Bool in clearStarted = true; return armed }
            if wait {
                await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
                    let resumeNow: Bool = locked {
                        if armed { gate = c; return false }
                        return true
                    }
                    if resumeNow { c.resume() }
                }
            }
            try await inner.clearRows()
        }
        func loadAllRows() async throws -> [String: [Row]] { try await inner.loadAllRows() }
        func loadCursor() async throws -> SyncCursor? { try await inner.loadCursor() }
        func saveCursor(_ cursor: SyncCursor) async throws { try await inner.saveCursor(cursor) }
        func persist(_ change: ChangeEvent) async throws { try await inner.persist(change) }
        func persistBatch(_ changes: [ChangeEvent], cursor: SyncCursor?) async throws {
            try await inner.persistBatch(changes, cursor: cursor)
        }
        func saveAll(_ mutations: [PendingMutation]) async throws { try await inner.saveAll(mutations) }
        func loadAll() async throws -> [PendingMutation] { try await inner.loadAll() }
    }

    private func makeEngine(
        _ server: Server,
        persistence: SyncPersistence? = nil
    ) async -> SyncEngine {
        let mock = MockTransport()
        mock.setHandler { [server] req in try await server.handle(req) }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: mock
        )
        await client.setSession(token: "tok1")
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client, persistence: persistence)
        await engine.refreshResolvedSession()
        await engine.pull()
        return engine
    }

    func testAFrameDuringAResetDoesNotLandInTheNewReplica() async throws {
        let path = NSTemporaryDirectory() + "pylon_reset_fence_\(UUID().uuidString).db"
        defer { try? FileManager.default.removeItem(atPath: path) }
        let persistence = try GatedPersistence(path: path)
        let server = Server()
        let engine = await makeEngine(server, persistence: persistence)
        persistence.arm()
        let reset = Task { await engine.resetReplica(wipeMutations: true) }
        while !persistence.started() { try await Task.sleep(nanoseconds: 1_000_000) }
        await engine.handleTextFrame(#"{"seq":900,"entity":"Secret","row_id":"s1","kind":"insert","data":{"id":"s1"}}"#)
        persistence.release()
        await reset.value
        await engine.waitForPersistIdle()
        let store = await engine.store
        XCTAssertTrue(store.list("Secret").isEmpty, "the old socket's frame must not survive the reset")
        let cursor = await engine.currentCursor().last_seq
        XCTAssertEqual(cursor, 0)
        let disk = try await persistence.inner.loadCursor()
        XCTAssertEqual(disk?.last_seq ?? 0, 0)
    }

    func testAnExpiredSessionHoldsWritesAndTheSameUserPushesThem() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await server.primePush([503])
        let id = try await engine.insert("Note", ["title": "offline draft"])
        var pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 1)

        await server.setAnonymous()
        await engine.notifySessionChanged()
        let user = await engine.currentResolvedSession().userId
        XCTAssertNil(user)
        pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 1, "an expired session keeps the offline writes")
        let pushesWhileOut = await server.pushCount()
        await engine.push()
        let pushesAfter = await server.pushCount()
        XCTAssertEqual(pushesAfter, pushesWhileOut, "no push while nobody is signed in")

        await server.setMe(userId: "u1", tenantId: nil)
        await engine.notifySessionChanged()
        try await Task.sleep(nanoseconds: 200_000_000)
        pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 0)
        let rows = await server.rows["Note"] ?? [:]
        XCTAssertNotNil(rows[id], "the same user signing back in pushes the held write")
        try await Task.sleep(nanoseconds: 1_200_000_000)
    }

    func testADifferentUserDiscardsTheHeldWrites() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await server.primePush([503])
        _ = try await engine.insert("Note", ["title": "u1 draft"])
        await server.setAnonymous()
        await engine.notifySessionChanged()
        await server.setMe(userId: "u2", tenantId: nil)
        await engine.notifySessionChanged()
        try await Task.sleep(nanoseconds: 1_200_000_000)
        let pending = await engine.mutations.all().count
        XCTAssertEqual(pending, 0)
        let rows = await server.rows["Note"] ?? [:]
        XCTAssertTrue(rows.isEmpty, "u1's write never pushes as u2")
    }

    func testAnOpWithNoResultResolvesAsQueued() async throws {
        let mock = MockTransport { req in
            switch req.url?.path ?? "" {
            case "/api/sync/push":
                return (200, Data(#"{"applied":0,"errors":[],"results":[{"op_id":"other","status":"applied"}],"cursor":{"last_seq":0}}"#.utf8))
            case "/api/sync/pull":
                return (200, Data(#"{"changes":[],"cursor":{"last_seq":0},"has_more":false}"#.utf8))
            default:
                return (200, Data(#"{"user_id":"u1","tenant_id":null,"is_admin":false,"roles":[]}"#.utf8))
            }
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(), transport: mock)
        let engine = await SyncEngine(
            config: SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60),
            client: client)
        let id = try await engine.insert("Note", ["title": "x"])
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: id))
        let pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 1)
    }
}
