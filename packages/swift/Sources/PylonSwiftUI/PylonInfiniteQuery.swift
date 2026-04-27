import Foundation
import PylonClient
import PylonSync

#if canImport(SwiftUI)
import SwiftUI

/// `ObservableObject` wrapper around `InfiniteQuery`. Mirrors the React
/// `useInfiniteQuery` hook — `loadMore()` extends the buffer; `rows`
/// updates trigger a SwiftUI re-render.
///
/// ```swift
/// @StateObject var feed = PylonInfiniteQuery<Post>(engine: engine, entity: "Post", pageSize: 25)
/// var body: some View {
///     List(feed.rows) { post in PostView(post: post) }
///     Button("Load more") { Task { try await feed.loadMore() } }
/// }
/// ```
@MainActor
public final class PylonInfiniteQuery<T: Decodable & Sendable>: ObservableObject {
    @Published public private(set) var rows: [T] = []
    @Published public private(set) var loading = false
    @Published public private(set) var hasMore = true
    @Published public private(set) var error: Error?

    private let query: InfiniteQuery<T>
    private var unsubscribe: (() -> Void)?

    public init(engine: SyncEngine, entity: String, pageSize: Int = 20) {
        let q = engine.createInfiniteQuery(entity, pageSize: pageSize, as: T.self)
        self.query = q
        Task { await self.start() }
    }

    deinit {
        unsubscribe?()
    }

    private func start() async {
        let cancel = await query.subscribe { [weak self] in
            Task { @MainActor in
                await self?.refresh()
            }
        }
        unsubscribe = cancel
        await loadMore()
    }

    private func refresh() async {
        rows = await query.data()
        hasMore = await query.hasMorePages()
        loading = await query.isLoading()
    }

    @discardableResult
    public func loadMore() async -> Bool {
        loading = true
        defer { loading = false }
        do {
            let newRows = try await query.loadMore()
            error = nil
            await refresh()
            return !newRows.isEmpty
        } catch {
            self.error = error
            return false
        }
    }

    public func reset() async {
        await query.reset()
        await refresh()
    }
}

/// Aggregate query (count, sum, avg, min, max, groupBy). Mirrors the React
/// `useAggregate` hook — re-runs whenever the underlying store notifies.
@MainActor
public final class PylonAggregate<Spec: Encodable & Sendable, Result: Decodable & Sendable>: ObservableObject {
    @Published public private(set) var result: Result?
    @Published public private(set) var loading = false
    @Published public private(set) var error: Error?

    private let engine: SyncEngine
    private let entity: String
    private let spec: Spec
    private var unsubscribe: (() -> Void)?

    public init(engine: SyncEngine, entity: String, spec: Spec) {
        self.engine = engine
        self.entity = entity
        self.spec = spec
        Task { await self.start() }
    }

    deinit { unsubscribe?() }

    private func start() async {
        let store = await engine.store
        let cancel = store.subscribe { [weak self] in
            Task { @MainActor in await self?.run() }
        }
        unsubscribe = cancel
        await run()
    }

    private func run() async {
        loading = true
        defer { loading = false }
        do {
            let r: Result = try await engine.client.aggregate(entity, spec)
            result = r
            error = nil
        } catch {
            self.error = error
        }
    }
}

/// Search query (FTS5-backed). Mirrors `useSearch`. Updates when `spec`
/// changes — call `refresh()` after mutating the spec field.
@MainActor
public final class PylonSearch<Spec: Encodable & Sendable, Result: Decodable & Sendable>: ObservableObject {
    @Published public private(set) var result: Result?
    @Published public private(set) var loading = false
    @Published public private(set) var error: Error?

    public var spec: Spec {
        didSet { Task { await self.refresh() } }
    }

    private let engine: SyncEngine
    private let entity: String

    public init(engine: SyncEngine, entity: String, spec: Spec) {
        self.engine = engine
        self.entity = entity
        self.spec = spec
        Task { await self.refresh() }
    }

    public func refresh() async {
        loading = true
        defer { loading = false }
        do {
            let r: Result = try await engine.client.search(entity, spec)
            result = r
            error = nil
        } catch {
            self.error = error
        }
    }
}
#endif
