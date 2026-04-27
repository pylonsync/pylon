import XCTest
import PylonClient
@testable import PylonSync

/// Atomic counter for tracking handler invocation order across the
/// `@Sendable` boundary that `MockTransport` requires.
final class TestCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = -1
    func increment() -> Int {
        lock.lock(); defer { lock.unlock() }
        value += 1
        return value
    }
}

final class InfiniteQueryTests: XCTestCase {

    func testLoadMoreAccumulatesAcrossPages() async throws {
        struct Todo: Codable, Equatable, Sendable { let id: String; let title: String }
        let transport = MockTransport()
        let pages: [(data: [Todo], next: String?, more: Bool)] = [
            (data: [Todo(id: "a", title: "first"), Todo(id: "b", title: "second")], next: "b", more: true),
            (data: [Todo(id: "c", title: "third")], next: nil, more: false),
        ]
        let counter = TestCounter()
        transport.setHandler { req in
            let i = counter.increment()
            let page = pages[i]
            let body: [String: Any] = [
                "data": page.data.map { ["id": $0.id, "title": $0.title] },
                "next_cursor": page.next as Any? ?? NSNull(),
                "has_more": page.more
            ]
            let data = try JSONSerialization.data(withJSONObject: body)
            return (200, data)
        }

        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        let cfg = SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll)
        let engine = await SyncEngine(config: cfg, client: client)
        let query: InfiniteQuery<Todo> = engine.createInfiniteQuery("Todo", pageSize: 2)

        // Page 1
        var loaded = try await query.loadMore()
        XCTAssertEqual(loaded.count, 2)
        var rows = await query.data()
        XCTAssertEqual(rows.map(\.id), ["a", "b"])

        // Page 2
        loaded = try await query.loadMore()
        XCTAssertEqual(loaded.count, 1)
        rows = await query.data()
        XCTAssertEqual(rows.map(\.id), ["a", "b", "c"])

        // No more pages
        let hasMore = await query.hasMorePages()
        XCTAssertFalse(hasMore)
        loaded = try await query.loadMore()
        XCTAssertTrue(loaded.isEmpty)
    }

    func testResetClearsBuffer() async throws {
        struct Todo: Codable, Equatable, Sendable { let id: String }
        let transport = MockTransport()
        transport.setHandler { _ in
            let body: [String: Any] = [
                "data": [["id": "a"]],
                "next_cursor": NSNull(),
                "has_more": false
            ]
            return (200, try JSONSerialization.data(withJSONObject: body))
        }
        let client = PylonClient(
            config: PylonClientConfig(baseURL: URL(string: "http://test.invalid")!),
            storage: MemoryStorage(),
            transport: transport
        )
        let engine = await SyncEngine(config: SyncEngineConfig(baseURL: URL(string: "http://test.invalid")!, transport: .poll), client: client)
        let query: InfiniteQuery<Todo> = engine.createInfiniteQuery("Todo")

        _ = try await query.loadMore()
        var rows = await query.data()
        XCTAssertEqual(rows.count, 1)

        await query.reset()
        rows = await query.data()
        XCTAssertEqual(rows.count, 0)
        let hasMore = await query.hasMorePages()
        XCTAssertTrue(hasMore, "reset must restore hasMore = true so loadMore works again")
    }
}
