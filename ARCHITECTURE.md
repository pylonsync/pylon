# Architecture

A map of the pylon codebase for new contributors. Pairs with [README.md](README.md)
(user-facing) and [SECURITY.md](SECURITY.md) (operational hardening).

## High-level shape

```
┌─────────────────────────────────────────────────────────────────┐
│                         Client (browser, RN, server)            │
│  packages/react · packages/sync · packages/sdk · packages/fns   │
└────────────────────────────┬────────────────────────────────────┘
                             │ HTTP + WS + SSE
┌────────────────────────────▼────────────────────────────────────┐
│  crates/cli  (pylon dev | build | migrate | seed | deploy)    │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│  crates/runtime  (tiny_http server, RoomManager, JobQueue,      │
│                   PubSub, WorkflowEngine, BunRuntime)           │
│        │              │              │              │           │
│        ▼              ▼              ▼              ▼           │
│   crates/router  crates/realtime  crates/auth   crates/policy   │
│   (route(),       (Shard, tick,    (sessions,   (rule engine,   │
│    DataStore-     SimState,        magic codes,  RBAC, expr     │
│    backed)        AOI, replay)     OAuth)        evaluator)     │
│                                                                 │
│  crates/sync   crates/storage   crates/plugin   crates/migrate  │
│  (change log)  (sqlite/pg)      (cache, etc.)   (schema diff)   │
│                                                                 │
│       crates/http  (DataStore trait — platform boundary)        │
└─────────────────────────────────────────────────────────────────┘
```

The boundary between platform-agnostic logic and the self-hosted server is
`crates/http`'s `DataStore` trait. The same `route()` function in
`crates/router` runs on top of SQLite, Postgres, or D1-compatible storage
adapters.

## Crate responsibilities

### Foundation

- **`crates/core`** — error codes (`ExitCode`, `AgentDBErrorCode`), shared
  types (`AppManifest`, `EntityDef`, `FieldType`), no I/O.
- **`crates/http`** — `HttpMethod`, `DataError`, and the `DataStore` trait.
  This is the contract every backend (SQLite, D1, Postgres) implements.

### Routing

- **`crates/router`** — `route(ctx, method, url, body, auth)`. Owns every URL
  the framework exposes: `/api/sync/*`, `/api/auth/*`, `/api/entity/*`,
  `/api/fn/*`, `/api/aggregate/*`, `/api/import`, `/api/rooms/*`,
  `/api/jobs/*`, `/api/workflows/*`, `/api/shards/*`, `/api/files/*`.
  Depends only on traits — no `tiny_http`, no `worker`.

### Storage

- **`crates/storage`** — SQL generation (`quote_ident`, `validate_column_name`,
  `create_table_sql`), SQLite adapter, Postgres adapter behind the
  `postgres-live` feature flag, file-storage helpers.
- **`crates/runtime/src/datastore.rs`** — `impl DataStore for Runtime` plus
  `TxStore` (the in-transaction handle passed to TS mutations).
- **`crates/migrate`** — schema diff engine. Compares current `AppManifest`
  to the live database, emits a plan, applies via `migrate apply`.

### Cross-cutting

- **`crates/auth`** — `SessionStore` + `SessionBackend` trait (in-memory or
  SQLite-backed via `crates/runtime/src/session_backend.rs`), `MagicCodeStore`
  with brute-force protection, `OAuthConfig` (Google + GitHub), `AuthContext`
  with role membership.
- **`crates/policy`** — expression evaluator. Supports `auth.userId`,
  `auth.isAdmin`, `auth.hasRole('x')`, `auth.hasAnyRole('a','b')`,
  `record.fieldName`, basic boolean and comparison ops.
- **`crates/sync`** — append-only change log + cursor-based pull endpoint.
- **`crates/plugin`** — plugin trait + builtins (cache, webhooks, soft delete,
  audit log).

### Compute

- **`crates/runtime`** — the self-hosted server.
  - `lib.rs` — `Runtime` (SQLite-backed; CRUD + `_with_conn` variants).
  - `server.rs` — `tiny_http` listener, request → `route()` glue, drain on
    shutdown, optional Bun spawn, optional shard WebSocket server (port + 3).
  - `config.rs` — `ServerConfig::from_env`. The single env-var entry point.
  - `rooms.rs`, `jobs.rs`, `workflows.rs`, `pubsub.rs` — multiplayer rooms,
    background queue, durable workflow engine, in-process pub/sub.
  - `bun_runtime.rs` — child process supervisor for the TS function runtime.
  - `session_backend.rs` — SQLite session persistence.
- **`crates/realtime`** — game/collab shards. `Shard<S: SimState>`, tick loop,
  area-of-interest, snapshot delta, replay, matchmaker. WebSocket transport
  in `transport.rs`.
- **`crates/functions`** — Rust side of the TS function runtime. NDJSON over
  stdin/stdout to a Bun child process.
- **`crates/workers`** — Cloudflare Workers adapter (experimental — see
  `crates/workers/README.md` for what works and what doesn't).

### CLI

- **`crates/cli`** — the `pylon` binary. `main.rs` dispatches to
  `commands/<verb>.rs`. JSON output mode for everything; structured logs to
  stderr via `init_tracing()`.

### TypeScript packages

- **`packages/sdk`** — schema DSL + manifest builder. Run during `pylon
  codegen` to emit `pylon.manifest.json`.
- **`packages/functions`** — `mutation(...)`, `query(...)`, `action(...)`
  helpers. The Bun runtime in `packages/functions/src/runtime.ts` reads NDJSON
  RPCs from stdin and dispatches to handlers.
- **`packages/react`** — `useQuery`, `useMutation`, `useInfiniteQuery`,
  `useFn`, `useShard`, plus `createTypedDb<S>()` for codegen-driven typing.
- **`packages/sync`** — client-side replica with optimistic mutations and an
  offline write queue. Server-authoritative LWW with field-level merge —
  not CRDT-backed, not local-first in the Ink & Switch sense; see
  [docs/SYNC.md](docs/SYNC.md) for what convergence guarantees this provides.
- **`packages/react-native`** — RN hooks + Expo SQLite-backed replica.
- **`packages/swift`** — native Swift SDK (iOS, macOS, Linux). `PylonClient`,
  `PylonSync` (LocalStore + MutationQueue + WebSocket + SQLite persistence),
  `PylonRealtime` (shard client), `PylonSwiftUI` (`@ObservableObject`
  helpers). Wire shapes pinned to TS; CRDT decoding via `loro-swift`
  (same Rust core as the JS Loro).
- **`packages/workflows`** — durable workflow runner (sidecar process).

## Request lifecycle (self-hosted)

```
HTTP request (tiny_http)
  → server.rs builds RouterContext (refs to Runtime, SessionStore, etc.)
  → router::route() dispatches by URL
  → handler reads/writes via DataStore trait (Runtime impl)
  → response built; CORS + security headers added in server.rs
  → tiny_http writes response
```

Mutations that hit `/api/fn/<name>` instead route to `BunRuntime`, which:

1. Sends `{call_id, fn, args, ctx}` as NDJSON to the Bun child.
2. Bun handler runs with a typed `ctx.db` proxy — every `db.insert/get/update`
   call round-trips back to Rust, where it's executed inside a single
   transaction (`TxStore`).
3. Bun returns `{call_id, result | error}`; Rust commits or rolls back.

The `pendingRpcs` map keyed by `call_id` is what makes concurrent function
calls safe with a single reader loop.

## Real-time

Two unrelated systems share the word "real-time":

- **Sync** (`crates/sync`) — invalidates client replicas. Server-authoritative,
  durable, append-only. Transport is HTTP pull + WebSocket/SSE hints.
- **Shards** (`crates/realtime`) — tick-driven multiplayer simulations.
  `Shard<S: SimState>` runs a fixed-rate game loop, broadcasts snapshot deltas
  to subscribers, accepts inputs through `push_input_json`. Each shard owns
  its state — no shared lock contention. Used by Mooncraft.

## Transactions

- HTTP mutations on entities are atomic per-request (single SQLite transaction
  in `Runtime`).
- TS function handlers are atomic per-call: the entire handler runs inside one
  `TxStore` transaction. Throwing rolls back; returning commits.
- Workflows are *not* atomic — they're durable step machines with retries.

## Where things are deliberately small

- No ORM. SQL is generated from the manifest and parameterized.
- No async runtime in the HTTP path. `tiny_http` is blocking, one OS thread
  per connection. Easier to reason about, easier to debug, fast enough for
  the target deployment (single VPS, < 10k req/s).
- The Bun runtime is one child process. Concurrency comes from JS's event
  loop and from running the function handler inside a Rust transaction —
  not from a worker pool.
- No service mesh, no gRPC, no message broker. The runtime is one binary.

## What's experimental

- `crates/workers` — D1 DataStore is partially implemented; routes don't all
  work; WebSocket via Durable Objects is sketched but not wired.

## Adding a new feature: where does it go?

| Need to... | Touch |
|---|---|
| Add a URL | `crates/router/src/lib.rs` (and `crates/http` if it needs new trait methods) |
| Add a CLI command | `crates/cli/src/commands/<verb>.rs`, register in `main.rs` |
| Add a storage backend | `impl DataStore for ...` in a new crate |
| Add a built-in plugin | `crates/plugin/src/builtin/` |
| Add a TS hook | `packages/react/src/` |
| Change schema diff behavior | `crates/migrate` |
| Add a new auth provider | `crates/auth/src/oauth.rs` |
