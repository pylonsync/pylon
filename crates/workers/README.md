# pylon-workers

**Status: experimental / unverified.** This crate has been built and its
unit tests pass, but it has never been deployed end-to-end to Cloudflare
Workers. Several APIs were written against the `worker` crate's documented
surface without live testing. Use at your own risk.

## What works

- **`D1DataStore`** — a `DataStore` implementation that generates
  SQLite-dialect SQL and executes it through a pluggable `D1Executor`
  trait. The SQL generation is unit-tested. Safe to use outside of Workers
  by implementing `D1Executor` against any SQLite-compatible backend.
- **`NoopAll`** — stub implementations of the router's service traits
  (rooms, cache, pubsub, jobs, scheduler, workflows, files, openapi) for
  platforms that don't offer those capabilities. Safe to use.
- **`durable_object` module** — helper primitives (`do_websocket_sink`,
  `persist_to_do_storage`, `restore_from_do_storage`,
  `register_do_subscriber`) + a JavaScript template for writing your own
  Durable Object class. These are library building blocks, not a working
  deployment.

## What is NOT verified

- **`handler` module (behind the `workers` feature).** Uses the `worker`
  crate's `#[event(fetch)]` macro and calls `futures::executor::block_on`
  inside the fetch handler. The block-on strategy almost certainly **does
  not work on `wasm32-unknown-unknown`** (single-threaded, no pool). This
  module has never been built with `--features workers` against the real
  `worker` crate.
- **End-to-end deploy.** Nobody has run `worker-build --release
  --features workers` or `wrangler deploy` against this crate.
- **The Durable Object handler integration.** The JS template is a
  starting point, not an integrated flow. Wiring the DO to the Rust
  `Shard` abstraction requires bindings that don't exist yet.

## What would be needed for a real Workers path

1. Replace `futures::executor::block_on` with genuinely async execution.
   Either:
   - Add an `AsyncDataStore` trait alongside the sync one
   - Or make `DataStore` itself async (affects every platform)
2. Actually build with `worker-build` and iterate until it compiles and
   deploys. Expect multiple API surprises with `worker` vs. what's documented.
3. Write an integration test that hits a deployed Worker from a test script
   and asserts behavior end-to-end.
4. Add Durable Object bindings so shards survive across DO instances and
   survive hibernation.
5. Swap the `ureq`-based OAuth and email HTTP clients for `fetch()`
   (available inside Workers) — `ureq` can't run in WASM.

## Using this crate today

The safe subset you can depend on:

```rust
use pylon_workers::{D1DataStore, D1Executor, NoopAll};

// Implement D1Executor for your own DB connection and get a working
// DataStore for free:
struct MyExecutor { /* ... */ }
impl D1Executor for MyExecutor { /* ... */ }

let store = D1DataStore::new(MyExecutor { /* ... */ }, manifest);
```

For deployment to actual Cloudflare Workers, we recommend waiting until
this crate has been marked stable. Until then, self-hosting (via the
`pylon-runtime` crate) is the supported path.

## Tracking

See [issue #TODO — file this] for the path to 1.0.
