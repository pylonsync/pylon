# pylon-swift

Native Swift SDK for [Pylon](https://github.com/pylonsync/pylon). Drop-in
parity with the TypeScript client (`@pylonsync/sync` + `@pylonsync/react`):
auth, entity CRUD, server functions, file uploads, real-time sync with an
offline-safe write queue, SwiftUI hooks, and Loro CRDT subscriptions via
[`loro-swift`](https://github.com/loro-dev/loro-swift).

## Platforms

| Target          | Min version |
| --------------- | ----------- |
| iOS             | 16          |
| macOS           | 13          |
| tvOS            | 16          |
| watchOS         | 9           |
| Linux           | Swift 5.9+ (uses `FoundationNetworking`) |

## Install

In `Package.swift`:

```swift
.package(url: "https://github.com/pylonsync/pylon.git", from: "0.3.0"),
```

Then per-target:

```swift
.target(name: "MyApp", dependencies: [
    .product(name: "PylonClient",   package: "pylon"),
    .product(name: "PylonSync",     package: "pylon"),
    .product(name: "PylonRealtime", package: "pylon"),
    .product(name: "PylonSwiftUI",  package: "pylon"),  // optional
])
```

Linux: `apt-get install libsqlite3-dev` for the SQLite-backed offline replica.

## Quickstart

```swift
import PylonClient
import PylonSync

// 1. HTTP client
let client = PylonClient(baseURL: URL(string: "http://localhost:4321")!)
try await client.startMagicCode(email: "alice@example.com")
_ = try await client.verifyMagicCode(email: "alice@example.com", code: "123456")

// 2. Sync engine — pulls + pushes + WebSocket reconnect with backoff
let cfg = SyncEngineConfig(baseURL: URL(string: "http://localhost:4321")!)
let persistence = try SQLitePersistence(path: NSHomeDirectory() + "/.pylon.db")
let engine = await SyncEngine(config: cfg, client: client, persistence: persistence)
await engine.start()

// 3. Optimistic mutations
_ = await engine.insert("Todo", ["title": "ship it", "done": false])

// 4. React to local-store changes
_ = engine.store.subscribe {
    let todos = engine.store.list("Todo")
    print("now have \(todos.count) todos")
}
```

## Codegen

Generate Codable structs + a typed `PylonClient` extension from your Pylon manifest:

```bash
pylon codegen client pylon.manifest.json --target swift --out Sources/MyApp/PylonGenerated.swift
```

After codegen, the typed helpers replace the stringly-typed APIs:

```swift
let todos: [Todo] = try await client.listTodos()
let created: Todo = try await client.createTodo(NewTodo(title: "x", done: false, ...))
try await client.deleteTodo(id: created.id)
```

## Modules

| Module          | What it gives you |
| --------------- | ----------------- |
| `PylonClient`   | HTTP client: auth, entities (CRUD + cursor pagination), `callFn`, `streamFn` (line-by-line via `URLSession.bytes`) + `streamFnBytes`, file upload/download, search, aggregate, `startSessionAutoRefresh(...)`, `getServerData(entities:)` SSR helper. Pluggable transport for tests. |
| `PylonSync`     | Sync engine, `LocalStore`, `MutationQueue`, transports (WebSocket / SSE fallback / poll) with bearer-token subprotocol and full-jitter exponential reconnect, `loadPage` + `InfiniteQuery` accumulator, SQLite persistence, Loro CRDT bridge. |
| `PylonRealtime` | Tick-driven shard client (`ShardClient<State, Input>`) for game/collab apps. |
| `PylonSwiftUI`  | `PylonQuery`, `PylonMutation`, `PylonSession`, `PylonInfiniteQuery`, `PylonAggregate`, `PylonSearch` — `ObservableObject` wrappers. |

## Transports

The sync engine supports three transports — pick via `SyncEngineConfig.transport`:

```swift
.websocket  // primary; bearer.<token> subprotocol; auto-reconnect with full-jitter backoff
.sse        // fallback; GET /events on port + 2; same backoff
.poll       // last resort; configurable interval
```

WebSocket and SSE both stream `ChangeEvent`s the engine applies to the
local replica. Switch transports without changing call sites.

## Streaming functions

```swift
// Line-by-line — perfect for NDJSON / SSE-flavored function output
for try await line in await client.streamFn("chat", args: ChatArgs(prompt: "...")) {
    print(line)
}

// Raw bytes — for binary streams
for try await chunk in await client.streamFnBytes("audio", args: ...) {
    speaker.feed(chunk)
}
```

## Architecture notes

- **Wire shapes** match the TS client byte-for-byte. JSON keys, frame
  layouts, and the WebSocket bearer-subprotocol scheme are pinned by
  shared regression tests.
- **CRDT logic is not reimplemented** — `loro-swift` wraps the same Rust
  Loro core that `loro-crdt` does on the JS side. Convergence semantics
  are identical by construction.
- **Sync engine** ports the TS engine's identity-flip detection,
  410-RESYNC circuit breaker, full-jitter exponential backoff, op_id
  idempotency, and tombstone-aware merge.
- **Offline writes**: `SQLitePersistence` mirrors the IndexedDB schema
  used by the web client (`packages/sync/src/persistence.ts`).
- **Foundation only on Linux** — `URLSession` (with continuation-based
  shims for `data(for:)` on `FoundationNetworking`), system libsqlite3,
  no Apple-only deps in the core targets. SwiftUI helpers are gated with
  `#if canImport(SwiftUI)`.

## Tests

```bash
cd packages/swift
swift test
```

## License

Same as Pylon: MIT OR Apache-2.0.
