import XCTest
import PylonClient
@testable import PylonSync

/// insert / update / delete report the server's verdict to the caller.
/// Parity with `packages/sync/src/mutation-outcome.test.ts`.
///
/// Before: a write the server refused was rolled back and marked failed,
/// but the call returned normally, so the app could not say "not sent".
final class MutationOutcomeTests: XCTestCase {
    typealias Server = SyncEngineParityP1Tests.Server

    private func makeEngine(_ server: Server, pushDelayNanos: UInt64 = 0) async -> SyncEngine {
        let mock = MockTransport()
        mock.setHandler { [server] req in
            if pushDelayNanos > 0, req.url?.path == "/api/sync/push" {
                try await Task.sleep(nanoseconds: pushDelayNanos)
            }
            return try await server.handle(req)
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: mock
        )
        await client.setSession(token: "tok1")
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client)
        await engine.refreshResolvedSession()
        await engine.pull()
        return engine
    }

    func testInsertThrowsTheServerErrorAndRemovesTheGhost() async throws {
        let server = Server()
        await server.deny("Message", code: "POLICY_DENIED", message: "not a member of this channel")
        let engine = await makeEngine(server)
        do {
            _ = try await engine.insert("Message", ["body": "hi"])
            XCTFail("insert must throw when the server rejects it")
        } catch let err as MutationRejectedError {
            XCTAssertEqual(err.code, "POLICY_DENIED")
            XCTAssertEqual(err.message, "not a member of this channel")
            XCTAssertEqual(err.entity, "Message")
            XCTAssertEqual(err.kind, .insert)
        }
        let store = await engine.store
        XCTAssertEqual(store.list("Message").count, 0)
    }

    func testUpdateAndDeleteThrowAndRestoreTheRow() async throws {
        let server = Server()
        await server.seed(ChangeEvent(seq: 1, entity: "Note", row_id: "n1", kind: .insert, data: ["title": "original"]))
        let engine = await makeEngine(server)
        await server.deny("Note", code: "VALIDATION_FAILED", message: "title too long")
        let store = await engine.store

        do {
            try await engine.update("Note", id: "n1", ["title": "x"])
            XCTFail("update must throw")
        } catch let err as MutationRejectedError {
            XCTAssertEqual(err.code, "VALIDATION_FAILED")
            XCTAssertEqual(err.kind, .update)
        }
        XCTAssertEqual(store.get("Note", id: "n1")?["title"]?.stringValue, "original")

        do {
            try await engine.delete("Note", id: "n1")
            XCTFail("delete must throw")
        } catch let err as MutationRejectedError {
            XCTAssertEqual(err.kind, .delete)
        }
        XCTAssertNotNil(store.get("Note", id: "n1"))
    }

    func testWholeRequestRejectionThrows() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await server.primePush([403])
        do {
            _ = try await engine.insert("Note", ["title": "x"])
            XCTFail("insert must throw on a 403 push")
        } catch let err as MutationRejectedError {
            XCTAssertEqual(err.code, "UNAVAILABLE")
        }
    }

    func testOfflineInsertReturnsAndKeepsTheGhost() async throws {
        let server = Server()
        let engine = await makeEngine(server)
        await server.primePush([503])
        let id = try await engine.insert("Note", ["title": "offline"])
        let store = await engine.store
        XCTAssertNotNil(store.get("Note", id: id))
        let pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 1)
        // Let the backoff retry land before the test tears down.
        try await Task.sleep(nanoseconds: 1_300_000_000)
    }

    func testAWriteQueuedDuringAPushStillReachesTheServer() async throws {
        let server = Server()
        let engine = await makeEngine(server, pushDelayNanos: 150_000_000)
        async let a = engine.insert("Note", ["title": "one"])
        try await Task.sleep(nanoseconds: 50_000_000)
        async let b = engine.insert("Note", ["title": "two"])
        let (ida, idb) = try await (a, b)
        let rows = await server.rows["Note"] ?? [:]
        XCTAssertNotNil(rows[ida])
        XCTAssertNotNil(rows[idb], "the second write joined a running push and must still be sent")
        let pending = await engine.mutations.pending().count
        XCTAssertEqual(pending, 0)
    }

    func testWipeFailsAWaitingWrite() async throws {
        let queue = MutationQueue()
        let id = await queue.add(ClientChange(entity: "Note", row_id: "n1", kind: .insert, data: [:]))
        async let wait: Void = queue.waitForOutcome(id)
        try await Task.sleep(nanoseconds: 20_000_000)
        await queue.wipeAll()
        do {
            try await wait
            XCTFail("a discarded write must fail its waiter")
        } catch let err as MutationRejectedError {
            XCTAssertEqual(err.code, "DISCARDED")
        }
    }
}
