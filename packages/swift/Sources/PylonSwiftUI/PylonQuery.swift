import Foundation
import PylonClient
import PylonSync

#if canImport(Combine)
import Combine
#endif

#if canImport(SwiftUI)
import SwiftUI

/// Subscribes to a Pylon entity and republishes its rows whenever the
/// underlying `LocalStore` changes. Mirrors the React `useQuery` hook.
///
/// ```swift
/// @StateObject var todos = PylonQuery<Todo>(engine: engine, entity: "Todo")
/// var body: some View {
///     List(todos.rows) { Text($0.title) }
/// }
/// ```
@MainActor
public final class PylonQuery<T: Decodable>: ObservableObject {
    @Published public private(set) var rows: [T] = []
    /// True while the first server snapshot is still in flight and we have no
    /// rows to show. A one-way latch (mirrors React `useQuery`): drops to false
    /// when rows arrive OR the engine's initial sync settles, then stays false.
    /// Gate skeletons on this instead of `rows.isEmpty` so a cold launch (or a
    /// post-org-switch resnapshot) doesn't flash the empty state for the
    /// seconds the snapshot takes.
    @Published public private(set) var loading = true
    @Published public private(set) var error: Error?

    private let engine: SyncEngine
    private let entity: String
    private let predicate: ((Row) -> Bool)?
    private var unsubscribe: (() -> Void)?
    private let decoder = JSONDecoder()

    public init(
        engine: SyncEngine,
        entity: String,
        where predicate: ((Row) -> Bool)? = nil
    ) {
        self.engine = engine
        self.entity = entity
        self.predicate = predicate
        Task { await self.start() }
    }

    deinit {
        unsubscribe?()
    }

    private func start() async {
        // Register this entity for reconcile sweeping so a server row in a
        // never-cached entity (and deletes the WS missed) converge into the
        // view, not just whatever happens to be in the local replica.
        await engine.observeEntity(entity)
        let store = await engine.store
        let cancel = store.subscribe { [weak self] in
            Task { @MainActor in
                self?.refresh()
            }
        }
        self.unsubscribe = cancel
        refresh()
    }

    private func refresh() {
        Task { @MainActor in
            let store = await engine.store
            let rows = store.list(entity)
            let filtered = predicate.map { p in rows.filter(p) } ?? rows
            // Drop `loading` once rows arrive OR the engine's initial sync
            // settles (server-confirmed, even if the result is empty). Gating on
            // isInitialSyncSettled() — not mere local hydration — is what keeps
            // a cold load showing a skeleton instead of the empty state until
            // the snapshot lands. One-way: never flips back to true here.
            // (Read the actor signal first — `await` can't live inside the `||`
            // short-circuit autoclosure.)
            if self.loading {
                let settled = await engine.isInitialSyncSettled()
                if !filtered.isEmpty || settled {
                    self.loading = false
                }
            }
            do {
                let decoded: [T] = try filtered.compactMap { row in
                    let data = try JSONEncoder().encode(row)
                    return try self.decoder.decode(T.self, from: data)
                }
                self.rows = decoded
                self.error = nil
            } catch {
                self.error = error
            }
        }
    }
}

/// Wraps a server function call as an observable command. Mirrors
/// `useMutation` from React.
///
/// ```swift
/// @StateObject var createTodo = PylonMutation<CreateArgs, Todo>(
///     client: client,
///     name: "createTodo"
/// )
/// Button("Add") { Task { try await createTodo.run(args) } }
/// ```
@MainActor
public final class PylonMutation<Args: Encodable & Sendable, Result: Decodable & Sendable>: ObservableObject {
    @Published public private(set) var loading = false
    @Published public private(set) var result: Result?
    @Published public private(set) var error: Error?

    private let client: PylonClient
    private let name: String

    public init(client: PylonClient, name: String) {
        self.client = client
        self.name = name
    }

    @discardableResult
    public func run(_ args: Args) async throws -> Result {
        loading = true
        defer { loading = false }
        do {
            let r: Result = try await client.callFn(name, args: args)
            self.result = r
            self.error = nil
            return r
        } catch {
            self.error = error
            throw error
        }
    }
}

/// Snapshot of `ResolvedSession` that re-publishes whenever the engine's
/// session state flips (sign-in, sign-out, tenant switch).
@MainActor
public final class PylonSession: ObservableObject {
    @Published public private(set) var session = ResolvedSession()

    private let engine: SyncEngine
    private var unsubscribe: (() -> Void)?

    public init(engine: SyncEngine) {
        self.engine = engine
        Task { await self.start() }
    }

    deinit {
        unsubscribe?()
    }

    private func start() async {
        let store = await engine.store
        let cancel = store.subscribe { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.session = await self.engine.currentResolvedSession()
            }
        }
        self.unsubscribe = cancel
        self.session = await engine.currentResolvedSession()
    }
}
#endif
