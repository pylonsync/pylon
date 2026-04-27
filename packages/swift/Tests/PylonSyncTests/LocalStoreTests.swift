import XCTest
import PylonClient
@testable import PylonSync

final class LocalStoreTests: XCTestCase {

    func testInsertUpdateDelete() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "do laundry", "done": false]))
        XCTAssertEqual(store.list("Todo").count, 1)
        XCTAssertEqual(store.get("Todo", id: "t1")?["title"]?.stringValue, "do laundry")

        store.applyChange(ChangeEvent(seq: 2, entity: "Todo", row_id: "t1", kind: .update, data: ["done": true]))
        XCTAssertEqual(store.get("Todo", id: "t1")?["done"]?.boolValue, true)
        XCTAssertEqual(store.get("Todo", id: "t1")?["title"]?.stringValue, "do laundry", "update should preserve unpatched fields")

        store.applyChange(ChangeEvent(seq: 3, entity: "Todo", row_id: "t1", kind: .delete))
        XCTAssertNil(store.get("Todo", id: "t1"))
    }

    func testTombstoneBlocksStaleResurrect() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Todo", row_id: "t1", kind: .delete))
        // Older insert should be dropped.
        store.applyChange(ChangeEvent(seq: 3, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "stale"]))
        XCTAssertNil(store.get("Todo", id: "t1"), "delete should win over older insert")

        // Newer insert (post-tombstone) should land.
        store.applyChange(ChangeEvent(seq: 7, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "fresh"]))
        XCTAssertEqual(store.get("Todo", id: "t1")?["title"]?.stringValue, "fresh")
    }

    func testInsertEnforcesAuthoritativeId() {
        let store = LocalStore()
        // Server bug / hostile event — payload claims id="evil" but row_id is "t1".
        store.applyChange(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["id": "evil", "title": "attempted overwrite"]))
        let row = store.get("Todo", id: "t1")
        XCTAssertEqual(row?["id"]?.stringValue, "t1", "row_id must override any id field in the payload")
    }

    func testOptimisticDeleteThenServerInsertStaysGone() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 5, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x"]))
        store.optimisticDelete("Todo", id: "t1")
        // Concurrent server replay of the original insert (older seq) — must be ignored.
        store.applyChange(ChangeEvent(seq: 4, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x-replay"]))
        XCTAssertNil(store.get("Todo", id: "t1"))
    }

    func testListenerFiresOnApply() {
        let store = LocalStore()
        var calls = 0
        let unsubscribe = store.subscribe { calls += 1 }
        store.applyChanges([ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x"])])
        XCTAssertEqual(calls, 1)
        unsubscribe()
        store.applyChanges([ChangeEvent(seq: 2, entity: "Todo", row_id: "t2", kind: .insert, data: ["title": "y"])])
        XCTAssertEqual(calls, 1, "after unsubscribe, listener stops firing")
    }

    func testClearAllDropsRowsAndTombstones() {
        let store = LocalStore()
        store.applyChange(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x"]))
        store.applyChange(ChangeEvent(seq: 2, entity: "Todo", row_id: "t2", kind: .delete))
        store.clearAll()
        XCTAssertEqual(store.list("Todo").count, 0)
        // Post-clear, an old-seq insert that the tombstone would have blocked must now land.
        store.applyChange(ChangeEvent(seq: 1, entity: "Todo", row_id: "t2", kind: .insert, data: ["title": "y"]))
        XCTAssertEqual(store.get("Todo", id: "t2")?["title"]?.stringValue, "y")
    }
}
