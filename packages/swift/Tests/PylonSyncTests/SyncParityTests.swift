import XCTest
import PylonClient
@testable import PylonSync

/// Regression tests for the TS↔Swift SyncEngine parity port: phantom-row
/// sweep (reconcile), optimistic rollback on a permanently-rejected push,
/// and the supporting store/queue primitives. Mirrors the behaviors in
/// `packages/sync/src/scenarios.test.ts` that the Swift engine was missing
/// (which surfaced as the yapless Mac app showing stale/ghost recordings).
final class SyncParityTests: XCTestCase {

    // MARK: - Push-rejection wire shape (P0-1)

    /// The server emits a rejected op's error as an OBJECT `{code, message}`.
    /// `PushOpResult.error` MUST decode that (and the legacy bare-string form),
    /// else the WHOLE `PushResponse` decode throws → the engine misreads it as
    /// a transient failure → rejected mutations retry forever and rollback
    /// never runs. This is the exact wire contract `/api/sync/push` produces.
    func testPushResponseDecodesObjectAndStringError() throws {
        let objectForm = """
        {"applied":1,"errors":[],"cursor":{"last_seq":7},
         "results":[
           {"op_id":"a","status":"applied","seq":7},
           {"op_id":"b","status":"error","error":{"code":"VALIDATION","message":"bad title"}}
         ]}
        """.data(using: .utf8)!
        let resp = try JSONDecoder().decode(PushResponse.self, from: objectForm)
        let rejected = resp.results?.first { $0.op_id == "b" }
        XCTAssertEqual(rejected?.status, "error")
        XCTAssertEqual(rejected?.error?.code, "VALIDATION")
        XCTAssertEqual(rejected?.error?.message, "bad title",
                       "object-form error must decode (whole response would throw pre-fix)")

        // Legacy bare-string form still decodes.
        let stringForm = """
        {"applied":0,"errors":["nope"],"cursor":{"last_seq":3},
         "results":[{"op_id":"c","status":"error","error":"nope"}]}
        """.data(using: .utf8)!
        let resp2 = try JSONDecoder().decode(PushResponse.self, from: stringForm)
        XCTAssertEqual(resp2.results?.first?.error?.message, "nope")
        XCTAssertNil(resp2.results?.first?.error?.code)
    }

    // MARK: - applyReconcileBatch (phantom-row sweep)

    func testReconcileBatchRemovesServerAbsentRow() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Recording", row_id: "r1", kind: .insert, data: ["title": "keep"]))
        store.applyChange(ChangeEvent(seq: 6, entity: "Recording", row_id: "r2", kind: .insert, data: ["title": "phantom"]))
        // Server now returns only r1 → r2 is a phantom and must be swept.
        store.applyReconcileBatch(
            "Recording",
            upserts: [["id": "r1", "title": "keep"]],
            removalIds: ["r2"],
            tombstoneSeq: 6
        )
        XCTAssertNotNil(store.get("Recording", id: "r1"))
        XCTAssertNil(store.get("Recording", id: "r2"), "server-absent row must be swept")
    }

    func testReconcileTombstoneBlocksStaleReplayButAllowsRecreate() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Recording", row_id: "r2", kind: .insert, data: ["title": "phantom"]))
        store.applyReconcileBatch("Recording", upserts: [], removalIds: ["r2"], tombstoneSeq: 10)
        XCTAssertNil(store.get("Recording", id: "r2"))
        // A stale replay at seq < tombstone is dropped...
        store.applyChange(ChangeEvent(seq: 8, entity: "Recording", row_id: "r2", kind: .insert, data: ["title": "stale"]))
        XCTAssertNil(store.get("Recording", id: "r2"))
        // ...but a genuine re-creation (higher seq) flows through.
        store.applyChange(ChangeEvent(seq: 12, entity: "Recording", row_id: "r2", kind: .insert, data: ["title": "recreated"]))
        XCTAssertEqual(store.get("Recording", id: "r2")?["title"]?.stringValue, "recreated")
    }

    // MARK: - Optimistic rollback on permanent rejection

    func testRollbackOptimisticInsertRemovesGhost() {
        let store = LocalStore()
        let tempId = store.optimisticInsert("Recording", ["title": "ghost"])
        XCTAssertNotNil(store.get("Recording", id: tempId))
        store.rollbackOptimisticInsert("Recording", id: tempId)
        XCTAssertNil(store.get("Recording", id: tempId), "rejected insert ghost must be removed")
    }

    func testRestoreRowBringsBackRejectedDeleteAndUnfences() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 1, entity: "Recording", row_id: "r1", kind: .insert, data: ["title": "important"]))
        let prev = store.get("Recording", id: "r1")
        store.optimisticDelete("Recording", id: "r1")
        XCTAssertNil(store.get("Recording", id: "r1"))
        // Permanent rejection → restore the row.
        store.restoreRow("Recording", id: "r1", prev: prev)
        XCTAssertEqual(store.get("Recording", id: "r1")?["title"]?.stringValue, "important", "rejected delete must restore the row")
        // The optimistic fence must clear so a later legitimate server event
        // for the id isn't dropped.
        store.applyChange(ChangeEvent(seq: 2, entity: "Recording", row_id: "r1", kind: .update, data: ["title": "edited"]))
        XCTAssertEqual(store.get("Recording", id: "r1")?["title"]?.stringValue, "edited", "fence must clear so future events apply")
    }

    func testRestoreRowRevertsRejectedUpdate() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 1, entity: "Recording", row_id: "r1", kind: .insert, data: ["title": "original"]))
        let prev = store.get("Recording", id: "r1")
        store.optimisticUpdate("Recording", id: "r1", ["title": "typo"])
        XCTAssertEqual(store.get("Recording", id: "r1")?["title"]?.stringValue, "typo")
        store.restoreRow("Recording", id: "r1", prev: prev)
        XCTAssertEqual(store.get("Recording", id: "r1")?["title"]?.stringValue, "original", "rejected update must revert")
    }

    // MARK: - MutationQueue prevRow + pendingRowKeys (reconcile protection)

    func testMutationQueueTracksPrevRowAndPendingKeys() async {
        let q = MutationQueue()
        _ = await q.add(
            ClientChange(entity: "Recording", row_id: "r1", kind: .update, data: ["x": 1]),
            prevRow: ["id": "r1", "x": 0]
        )
        _ = await q.add(ClientChange(entity: "Recording", row_id: "r2", kind: .insert, data: ["x": 2]))
        let keys = await q.pendingRowKeys()
        XCTAssertTrue(keys.contains("Recording/r1"))
        XCTAssertTrue(keys.contains("Recording/r2"))
        let pending = await q.pending()
        let r1 = pending.first { $0.change.row_id == "r1" }
        XCTAssertEqual(r1?.prevRow?["x"]?.intValue, 0, "prevRow captured for rollback")
    }
}
