import XCTest
import PylonClient
@testable import PylonSync

/// Every queued write carries the user it was made as, and push only sends
/// it as that user. Parity with `packages/sync/src/mutation-owner.test.ts`.
final class MutationOwnerTests: XCTestCase {
    typealias Server = SyncEngineParityP1Tests.Server

    /// Answers /api/auth/me with a network error while `failMe` is set.
    final class Switch: @unchecked Sendable {
        private let lock = NSLock()
        private var _failMe = false
        var failMe: Bool {
            get { lock.lock(); defer { lock.unlock() }; return _failMe }
            set { lock.lock(); _failMe = newValue; lock.unlock() }
        }
    }

    private func makeEngine(
        _ server: Server,
        failSwitch: Switch? = nil,
        pullGate: (@Sendable () async -> Void)? = nil,
        client clientOut: ((PylonClient) -> Void)? = nil,
        persistence: SyncPersistence? = nil
    ) async -> SyncEngine {
        let mock = MockTransport()
        mock.setHandler { [server] req in
            let path = req.url?.path ?? ""
            if path == "/api/auth/me", failSwitch?.failMe == true {
                throw URLError(.notConnectedToInternet)
            }
            if path == "/api/sync/pull", let pullGate { await pullGate() }
            return try await server.handle(req)
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: mock
        )
        await client.setSession(token: "tok1")
        clientOut?(client)
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        return await SyncEngine(config: cfg, client: client, persistence: persistence)
    }

    func testAWriteIsTaggedWithItsUser() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await engine.refreshResolvedSession()
        await server.primePush([503])
        _ = try await engine.insert("Note", ["title": "x"])
        let m = await engine.mutations.pending().first
        XCTAssertEqual(m?.owner, "u1")
        XCTAssertEqual(m?.ownerKnown, true)
        try await Task.sleep(nanoseconds: 1_200_000_000)
    }

    func testATokenChangeWhoseRefreshFailsRetriesUntilTheWritesPush() async throws {
        let server = Server()
        let failSwitch = Switch()
        var clientRef: PylonClient?
        let engine = await makeEngine(server, failSwitch: failSwitch, client: { clientRef = $0 })
        await engine.start()
        failSwitch.failMe = true
        await clientRef!.setSession(token: "tok2") // rotation, same user
        await engine.pull()
        let id = try await engine.insert("Note", ["title": "after rotation"])
        var pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 1, "held until the token's owner is known")
        failSwitch.failMe = false
        for _ in 0..<60 {
            pending = await engine.mutations.pending().count
            if pending == 0 { break }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        XCTAssertEqual(pending, 0)
        let rows = await server.rows["Note"] ?? [:]
        XCTAssertNotNil(rows[id])
        await engine.stop()
    }

    func testAfterAReloadWhileSignedOutThePreviousUsersWritesAreNotPushed() async throws {
        let path = NSTemporaryDirectory() + "pylon_owner_\(UUID().uuidString).db"
        defer { try? FileManager.default.removeItem(atPath: path) }
        let disk = try SQLitePersistence(path: path)
        var op = PendingMutation(
            id: "op-1",
            change: ClientChange(entity: "Note", row_id: "n1", kind: .insert, data: ["id": "n1"], op_id: "op-1"))
        op.owner = "u1"
        op.ownerKnown = true
        try await disk.saveAll([op])

        let server = Server()
        await server.setAnonymous()
        let engine = await makeEngine(server, persistence: disk)
        await engine.start()
        await engine.push()
        let pending = await engine.mutations.pending()
        XCTAssertEqual(pending.count, 1)
        let pushes = await server.pushCount()
        XCTAssertEqual(pushes, 0, "u1's write is never pushed as the anonymous caller")
        await engine.stop()
    }

    func testAPushAfterATokenSwapHoldsTheOldUsersWrite() async throws {
        let server = Server()
        var clientRef: PylonClient?
        let engine = await makeEngine(server, client: { clientRef = $0 })
        await engine.refreshResolvedSession()
        await server.primePush([503])
        let id = try await engine.insert("Note", ["title": "u1 draft"])
        // Another token is stored; nothing has refreshed the session yet.
        await clientRef!.setSession(token: "tok-other")
        let before = await server.pushCount()
        await engine.push()
        let after = await server.pushCount()
        XCTAssertEqual(after, before, "u1's write is not sent with another token")
        // The refresh says the token is u2's: u1's write is discarded.
        await server.setMe(userId: "u2", tenantId: nil)
        await engine.notifySessionChanged()
        try await Task.sleep(nanoseconds: 1_300_000_000)
        let rows = await server.rows["Note"] ?? [:]
        XCTAssertNil(rows[id])
        let left = await engine.mutations.all().count
        XCTAssertEqual(left, 0)
    }

    func testAWriteBeforeAnyIdentityWaitsAndTakesTheFirstSessionsOwner() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await server.primePush([])
        let insert = Task { try await engine.insert("Note", ["title": "early"]) }
        let id = try await insert.value
        let early = await engine.mutations.pending().first
        XCTAssertEqual(early?.ownerPending, true)
        var rows = await server.rows["Note"] ?? [:]
        XCTAssertNil(rows[id], "held until the session is known")
        await engine.refreshResolvedSession()
        await engine.push()
        rows = await server.rows["Note"] ?? [:]
        XCTAssertNotNil(rows[id])
    }

    func testOverlappingDeltaPullsKeepEachOthersHeldFrames() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "a"]))
        let gate = PullGate()
        let engine = await makeEngine(server, pullGate: { await gate.wait() })
        await engine.refreshResolvedSession()
        await engine.pull()
        await server.seed(ChangeEvent(seq: 6, entity: "Note", row_id: "n2", kind: .insert, data: ["title": "b"]))
        gate.block()
        let p1 = Task { await engine.pull() }
        try await Task.sleep(nanoseconds: 30_000_000)
        let p2 = Task { await engine.pull() }
        try await Task.sleep(nanoseconds: 30_000_000)
        // P1 lands first; P2 still owns the hold.
        gate.releaseOne()
        await p1.value
        // A write lands on the server after P1's page and before P2's.
        await server.seed(ChangeEvent(seq: 7, entity: "Note", row_id: "n3", kind: .insert, data: ["title": "c"]))
        // This frame arrives while P2 is in flight: it must wait for P2,
        // not apply now, move the cursor to 8, and make P2 drop seq 7.
        await engine.handleTextFrame(#"{"seq":8,"entity":"Note","row_id":"n4","kind":"insert","data":{"id":"n4"}}"#)
        gate.release()
        await p2.value
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: "n2"))
        XCTAssertNotNil(store.get("Note", id: "n3"), "P2's page must not be dropped")
        XCTAssertNotNil(store.get("Note", id: "n4"))
    }

    func testAnOverlappingPullDoesNotCloseTheNewPullsHold() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 5, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "a"]))
        let gate = PullGate()
        let engine = await makeEngine(server, pullGate: { await gate.wait() })
        await engine.refreshResolvedSession()
        gate.block()
        let p1 = Task { await engine.pull() }
        try await Task.sleep(nanoseconds: 30_000_000)
        // A reset (identity / 410) starts a second pull while the first
        // is still in flight.
        await engine.resetReplica()
        let p2 = Task { await engine.pull() }
        try await Task.sleep(nanoseconds: 30_000_000)
        // Release the first pull only: it sees the reset and returns.
        gate.releaseOne()
        await p1.value
        // A frame while the second pull is still in flight must be held by
        // it. If the first pull's exit closed the hold, the frame applies
        // now, moves the cursor to 6, and the snapshot row at seq 5 is
        // dropped.
        await engine.handleTextFrame(#"{"seq":6,"entity":"Note","row_id":"n2","kind":"insert","data":{"id":"n2"}}"#)
        gate.release()
        await p2.value
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: "n1"), "the second pull's snapshot landed")
        XCTAssertNotNil(store.get("Note", id: "n2"))
    }
}

/// Blocks pulls while `block()` is in effect.
final class PullGate: @unchecked Sendable {
    private let lock = NSLock()
    private var blocked = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    func block() { lock.lock(); blocked = true; lock.unlock() }
    func releaseOne() {
        lock.lock()
        let c = waiters.isEmpty ? nil : waiters.removeFirst()
        lock.unlock()
        c?.resume()
    }
    func release() {
        lock.lock()
        blocked = false
        let w = waiters
        waiters = []
        lock.unlock()
        for c in w { c.resume() }
    }
    func wait() async {
        await withCheckedContinuation { (c: CheckedContinuation<Void, Never>) in
            lock.lock()
            if blocked {
                waiters.append(c)
                lock.unlock()
            } else {
                lock.unlock()
                c.resume()
            }
        }
    }
}
