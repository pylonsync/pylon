import Foundation
import PylonClient

/// Cursor-paginated accumulator. Mirrors the TS `createInfiniteQuery`
/// from `packages/sync/src/index.ts`. Calls `client.listCursor(...)`
/// page by page; appends to an in-memory buffer; notifies subscribers
/// after each successful append.
///
/// Reset state with `reset()` when the underlying query changes.
public actor InfiniteQuery<T: Decodable & Sendable> {
    public let entity: String
    public let pageSize: Int
    private let client: PylonClient

    private var rows: [T] = []
    private var cursor: String? = nil
    private var hasMore: Bool = true
    private var loading: Bool = false
    private var listeners: [UUID: @Sendable () -> Void] = [:]

    init(client: PylonClient, entity: String, pageSize: Int) {
        self.client = client
        self.entity = entity
        self.pageSize = pageSize
    }

    public func data() -> [T] { rows }
    public func isLoading() -> Bool { loading }
    public func hasMorePages() -> Bool { hasMore }

    /// Load the next page if any. Returns the page that was appended.
    /// No-op if a load is already in flight or all pages have been
    /// consumed.
    @discardableResult
    public func loadMore() async throws -> [T] {
        if loading || !hasMore { return [] }
        loading = true
        defer { loading = false }
        let page: CursorPage<T> = try await client.listCursor(entity, after: cursor, limit: pageSize)
        rows.append(contentsOf: page.data)
        cursor = page.next_cursor
        hasMore = page.has_more
        notify()
        return page.data
    }

    /// Drop accumulated rows and reset the cursor.
    public func reset() {
        rows.removeAll()
        cursor = nil
        hasMore = true
        notify()
    }

    @discardableResult
    public func subscribe(_ listener: @escaping @Sendable () -> Void) -> () -> Void {
        let id = UUID()
        listeners[id] = listener
        return { [weak self] in
            Task { await self?.removeListener(id: id) }
        }
    }

    private func removeListener(id: UUID) {
        listeners.removeValue(forKey: id)
    }

    private func notify() {
        for l in listeners.values { l() }
    }
}
