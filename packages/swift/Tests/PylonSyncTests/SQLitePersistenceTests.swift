import XCTest
import PylonClient
@testable import PylonSync

final class SQLitePersistenceTests: XCTestCase {

    private func tempPath() -> String {
        let dir = NSTemporaryDirectory()
        return dir + "pylon_swift_test_\(UUID().uuidString).db"
    }

    func testCursorRoundTrip() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let p = try SQLitePersistence(path: path)
        try await p.saveCursor(SyncCursor(last_seq: 42))
        let loaded = try await p.loadCursor()
        XCTAssertEqual(loaded?.last_seq, 42)
    }

    func testRowPersistAndLoad() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let p = try SQLitePersistence(path: path)
        try await p.persist(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "ship", "done": false]))
        try await p.persist(ChangeEvent(seq: 2, entity: "Todo", row_id: "t2", kind: .insert, data: ["title": "review", "done": false]))
        try await p.persist(ChangeEvent(seq: 3, entity: "Note", row_id: "n1", kind: .insert, data: ["body": "hello"]))

        let loaded = try await p.loadAllRows()
        XCTAssertEqual(loaded["Todo"]?.count, 2)
        XCTAssertEqual(loaded["Note"]?.count, 1)
        XCTAssertEqual(loaded["Note"]?.first?["body"]?.stringValue, "hello")
    }

    func testDeletePersists() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let p = try SQLitePersistence(path: path)
        try await p.persist(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "x"]))
        try await p.persist(ChangeEvent(seq: 2, entity: "Todo", row_id: "t1", kind: .delete))
        let loaded = try await p.loadAllRows()
        XCTAssertNil(loaded["Todo"]?.first { $0["id"]?.stringValue == "t1" })
    }

    func testMutationQueueRoundTrip() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        let p = try SQLitePersistence(path: path)
        let m1 = PendingMutation(id: "mut_a", change: ClientChange(entity: "Todo", row_id: "t1", kind: .insert, op_id: "mut_a"))
        let m2 = PendingMutation(id: "mut_b", change: ClientChange(entity: "Todo", row_id: "t2", kind: .delete, op_id: "mut_b"), status: .failed, error: "boom")
        try await p.saveAll([m1, m2])
        let loaded = try await p.loadAll()
        XCTAssertEqual(loaded.count, 2)
        XCTAssertEqual(loaded.first(where: { $0.id == "mut_b" })?.status, .failed)
        XCTAssertEqual(loaded.first(where: { $0.id == "mut_b" })?.error, "boom")
    }

    func testReopenPersistsState() async throws {
        let path = tempPath()
        defer { try? FileManager.default.removeItem(atPath: path) }
        do {
            let p = try SQLitePersistence(path: path)
            try await p.saveCursor(SyncCursor(last_seq: 99))
            try await p.persist(ChangeEvent(seq: 1, entity: "Todo", row_id: "t1", kind: .insert, data: ["title": "persisted"]))
        }
        // Reopen — same file.
        let p2 = try SQLitePersistence(path: path)
        let cursor = try await p2.loadCursor()
        XCTAssertEqual(cursor?.last_seq, 99)
        let rows = try await p2.loadAllRows()
        XCTAssertEqual(rows["Todo"]?.first?["title"]?.stringValue, "persisted")
    }
}
