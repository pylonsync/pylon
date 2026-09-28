import XCTest
import PylonClient
@testable import PylonSync

/// #315 parity (TS `_initialSyncSettled` in packages/sync/src/index.ts): a
/// query's `loading` must stay true through the window where the local cache
/// has hydrated (possibly EMPTY) but the first SERVER pull hasn't landed —
/// otherwise a cold launch or a post-`resetReplica` org switch flashes the empty
/// state for the seconds the snapshot takes. The engine exposes
/// `isInitialSyncSettled()`; `PylonQuery.loading` gates on it. This pins the
/// engine signal's full lifecycle: false → (first pull) → true → (resetReplica)
/// → false → (re-pull) → true.
///
/// `MockTransport` is defined in SyncEngineIntegrationTests.swift (same test
/// target), so it's reused here.
final class InitialSyncSettledTests: XCTestCase {

    private func makeEngine() async -> SyncEngine {
        let transport = MockTransport { req in
            let path = req.url?.path ?? ""
            if path == "/api/auth/me" {
                let body: [String: Any] = [
                    "user_id": "u1", "tenant_id": NSNull(), "is_admin": false, "roles": [],
                ]
                return (200, try JSONSerialization.data(withJSONObject: body))
            }
            if path == "/api/sync/pull" {
                // An EMPTY but successful pull — exactly the case that used to
                // flash the empty state: the snapshot lands with no rows, and
                // the signal must settle so `loading` can drop to false.
                let resp: [String: Any] = [
                    "changes": [], "cursor": ["last_seq": 0], "has_more": false,
                ]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            }
            return (404, Data("{}".utf8))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        await client.setSession(token: "tok")
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        return await SyncEngine(config: cfg, client: client)
    }

    func testInitialSyncSettledLifecycle() async throws {
        let engine = await makeEngine()

        // Before any pull: NOT settled — a query must render its skeleton, not
        // the empty state.
        var settled = await engine.isInitialSyncSettled()
        XCTAssertFalse(settled, "must start unsettled so a cold load shows a skeleton")

        // First successful pull settles it (server-confirmed, even though the
        // result is empty).
        await engine.pull()
        settled = await engine.isInitialSyncSettled()
        XCTAssertTrue(settled, "first successful pull must settle the signal")

        // resetReplica (org switch / identity flip) wipes the replica and
        // re-enters loading until the re-pull lands.
        await engine.resetReplica()
        settled = await engine.isInitialSyncSettled()
        XCTAssertFalse(settled, "resetReplica must re-enter loading until the re-pull lands")

        // The re-pull re-settles it.
        await engine.pull()
        settled = await engine.isInitialSyncSettled()
        XCTAssertTrue(settled, "re-pull after resetReplica must re-settle the signal")

        // Cancel any armed fallback task so it doesn't outlive the test.
        await engine.stop()
    }

    /// `isSynced()` has no fallback and is not set by a warm cache: it flips
    /// only when a pull completes, and a reset clears it. Mirrors TS
    /// `packages/sync/src/synced-flag.test.ts`.
    func testSyncedFollowsCompletedPullsOnly() async throws {
        let engine = await makeEngine()
        var synced = await engine.isSynced()
        XCTAssertFalse(synced)
        await engine.pull()
        synced = await engine.isSynced()
        XCTAssertTrue(synced, "a completed pull marks the replica synced")
        await engine.resetReplica()
        synced = await engine.isSynced()
        XCTAssertFalse(synced, "a reset clears it until the re-pull")
        await engine.pull()
        synced = await engine.isSynced()
        XCTAssertTrue(synced)
        await engine.stop()
    }

    func testSyncedStaysFalseWhenThePullFails() async throws {
        let transport = MockTransport { req in
            if req.url?.path == "/api/sync/pull" { return (503, Data("{}".utf8)) }
            return (404, Data("{}".utf8))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        let cfg = SyncEngineConfig(
            baseURL: URL(string: "http://test.invalid")!, transport: .poll, pollInterval: 60)
        let engine = await SyncEngine(config: cfg, client: client)
        await engine.pull()
        let synced = await engine.isSynced()
        XCTAssertFalse(synced, "an unreachable server never marks the replica synced")
        await engine.stop()
    }
}
