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
