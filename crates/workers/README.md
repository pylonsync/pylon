# pylon-workers

**Status: compiles, never run.** `.github/workflows/ci.yml` runs
`cargo check -p pylon-workers --features workers --target wasm32-unknown-unknown`
on every Rust change, and it is a required job — so the crate type-checks
against the real `worker` crate on the real target. Nothing here has ever
executed. No `worker-build`, no `wrangler deploy`, no live request.

The gap between "type-checks" and "works" is load-bearing here. See
[The `block_on` problem](#the-block_on-problem).

## What's in the crate

| Module | What it is |
| --- | --- |
| `d1_store` | `DataStore` over D1. SQLite-dialect SQL generation behind a pluggable `D1Executor` trait. SQL generation is unit-tested and usable outside Workers against any SQLite-compatible backend. |
| `rooms_do` | `PylonRoom` Durable Object + the `WorkersRooms` `RoomOps` adapter. Rooms map 1:1 onto DOs via `id_from_name`; sockets are accepted with `state.accept_web_socket()` so they survive hibernation. |
| `pubsub_do` | DO-backed pub/sub fan-out. |
| `durable_object` | Helper primitives (`do_websocket_sink`, `persist_to_do_storage`, `restore_from_do_storage`, `register_do_subscriber`) + a JS class template. |
| `kv_cache` | `CacheOps` over Workers KV. |
| `r2_files` | File storage over R2. |
| `queue_jobs` | Job queue over Cloudflare Queues, with a KV-backed registry and DLQ. |
| `handler` | The `#[event(fetch)]` entry point wiring the router to a Worker request. |
| `noop_adapters` | `NoopAll` — stubs for the router's service traits (rooms, cache, pubsub, jobs, scheduler, workflows, files, openapi) on platforms lacking them. Safe to use anywhere. |

## The `block_on` problem

Roughly 30 call sites across `kv_cache`, `queue_jobs`, `r2_files`, `handler`,
and the `RoomOps` adapters in `rooms_do` / `pubsub_do` call
`futures::executor::block_on` to bridge the router's sync service traits to
the `worker` crate's async API.

On `wasm32-unknown-unknown` there is no thread to park and no way to yield
back to the JS event loop, so a pending JS promise never resolves. These call
sites **compile and then hang on first use.** `cargo check` cannot catch a
runtime deadlock, which is why CI is green.

The comment at `rooms_do.rs:27` claiming Workers' single-threaded runtime
tolerates this inside request handlers is wrong.

The fix is to make the trait boundary async — `DataStore`, `RoomOps`,
`CacheOps` — which affects every platform, not just Workers.

**Scope note:** the Durable Object *classes* are already fully async and free
of `block_on` (`rooms_do.rs:88-288`). Only the Worker-side adapters are
affected. A design that uses DOs without running the router on Workers does
not need this refactor. See
[`docs/SYNC_DURABLE_OBJECTS_DESIGN.md`](../../docs/SYNC_DURABLE_OBJECTS_DESIGN.md).

## What a full Workers deployment would still need

1. Async trait boundary, per above.
2. An actual `worker-build --release --features workers` + `wrangler deploy`.
   Expect API surprises between the `worker` crate and its docs.
3. An integration test that hits a deployed Worker and asserts behavior.
   `cargo check` is not evidence.
4. DO bindings so shards survive across instances and hibernation.
5. `fetch()` in place of the `ureq`-based OAuth and email HTTP clients —
   `ureq` cannot run in WASM.
6. D1 is the only SQL backend here. Apps on `postgres-live` have no path.

## Using this crate today

The safe subset:

```rust
use pylon_workers::{D1DataStore, D1Executor, NoopAll};

// Implement D1Executor for your own DB connection and get a working
// DataStore for free:
struct MyExecutor { /* ... */ }
impl D1Executor for MyExecutor { /* ... */ }

let store = D1DataStore::new(MyExecutor { /* ... */ }, manifest);
```

Self-hosting via `pylon-runtime` is the supported deployment path.
