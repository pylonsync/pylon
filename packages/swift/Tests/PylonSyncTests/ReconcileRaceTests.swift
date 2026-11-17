import XCTest
import PylonClient
@testable import PylonSync

/// Change events that arrive while a reconcile fetch is in flight. Parity
/// with "a reconcile whose fetch raced change events" in
/// `packages/sync/src/reconcile.test.ts`.
///
/// The reconcile used to discard a fetch when any change event arrived
/// during it. An entity that is written all the time (an append-only log)
/// never reconciled. Now the fetch applies, and only the rows those events
/// wrote are left alone: the fetch may have read them before the event.
final class ReconcileRaceTests: XCTestCase {

    final class EngineBox: @unchecked Sendable {
        private let lock = NSLock()
        private var engine: SyncEngine?
        func set(_ e: SyncEngine) { lock.lock(); engine = e; lock.unlock() }
        func get() -> SyncEngine? { lock.lock(); defer { lock.unlock() }; return engine }
    }

    final class Counter: @unchecked Sendable {
        private let lock = NSLock()
        private var n = 0
        func bump() { lock.lock(); n += 1; lock.unlock() }
        var value: Int { lock.lock(); defer { lock.unlock() }; return n }
    }

    /// An engine whose server answers `rows` for a Folder reconcile fetch,
    /// after delivering `frames` to the engine as live change events.
    private func makeEngine(
        rows: [[String: Any]],
        frames: [String],
        fetches: Counter
    ) async -> SyncEngine {
        let box = EngineBox()
        let mock = MockTransport()
        mock.setHandler { req in
            let path = req.url?.path ?? ""
            switch path {
            case "/api/auth/me":
                let body: [String: Any] = [
                    "user_id": "u1", "tenant_id": NSNull(), "is_admin": false, "roles": [],
                ]
                return (200, try JSONSerialization.data(withJSONObject: body))
            case "/api/sync/pull":
                let resp: [String: Any] = ["changes": [], "cursor": ["last_seq": 0], "has_more": false]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            case "/api/entities/Folder/cursor":
                fetches.bump()
                if let engine = box.get() {
                    for frame in frames { await engine.handleTextFrame(frame) }
                }
                let resp: [String: Any] = ["data": rows, "has_more": false, "next_cursor": NSNull()]
                return (200, try JSONSerialization.data(withJSONObject: resp))
            default:
                return (404, Data("{}".utf8))
            }
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
        box.set(engine)
        return engine
    }

    private func frame(_ seq: Int, _ kind: String, _ id: String, name: String? = nil) -> String {
        let data = name.map { #","data":{"id":"\#(id)","name":"\#($0)"}"# } ?? ""
        return #"{"seq":\#(seq),"entity":"Folder","row_id":"\#(id)","kind":"\#(kind)"\#(data)}"#
    }

    private func names(_ engine: SyncEngine) async -> [String: String] {
        let store = await engine.store
        var out: [String: String] = [:]
        for row in store.list("Folder") {
            guard case let .string(id)? = row["id"] else { continue }
            if case let .string(name)? = row["name"] { out[id] = name } else { out[id] = "" }
        }
        return out
    }

    func testAFetchThatRacedAChangeAppliesAtOnce() async throws {
        let fetches = Counter()
        let engine = await makeEngine(
            rows: [["id": "f1", "name": "Team"]],
            frames: [frame(1, "insert", "other", name: "Other")],
            fetches: fetches)
        await engine.reconcile(["Folder"])
        let got = await names(engine)
        XCTAssertEqual(got, ["f1": "Team", "other": "Other"])
        XCTAssertEqual(fetches.value, 1)
    }

    func testARowAnEventUpdatedDuringTheFetchKeepsTheEventsValue() async throws {
        let fetches = Counter()
        // The server read f1 before the update, so the fetch carries the old name.
        let engine = await makeEngine(
            rows: [["id": "f1", "name": "Old"], ["id": "f2", "name": "Second"]],
            frames: [frame(2, "update", "f1", name: "New")],
            fetches: fetches)
        await engine.handleTextFrame(frame(1, "insert", "f1", name: "Old"))
        await engine.reconcile(["Folder"])
        let got = await names(engine)
        XCTAssertEqual(got, ["f1": "New", "f2": "Second"])
    }

    func testARowAnEventInsertedDuringTheFetchIsNotRemovedAndAStaleRowIs() async throws {
        let fetches = Counter()
        // The insert landed after the server read past its position.
        let engine = await makeEngine(
            rows: [],
            frames: [frame(2, "insert", "fresh", name: "Fresh")],
            fetches: fetches)
        await engine.handleTextFrame(frame(1, "insert", "stale", name: "Deleted on the server"))
        await engine.reconcile(["Folder"])
        let got = await names(engine)
        XCTAssertEqual(got, ["fresh": "Fresh"])
    }

    func testARowAnEventDeletedDuringTheFetchIsNotBroughtBack() async throws {
        let fetches = Counter()
        // The server read f1 before the delete.
        let engine = await makeEngine(
            rows: [["id": "f1", "name": "Team"]],
            frames: [frame(2, "delete", "f1")],
            fetches: fetches)
        await engine.handleTextFrame(frame(1, "insert", "f1", name: "Team"))
        await engine.reconcile(["Folder"])
        let got = await names(engine)
        XCTAssertEqual(got, [:])
    }
}
