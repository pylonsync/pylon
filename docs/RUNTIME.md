# Pylon: Function Runtime

How TypeScript functions actually execute in Pylon — and an honest evaluation
of whether the current choice is the right one.

## TL;DR

- **Engine:** Bun, run as a single child process per Pylon instance.
- **Protocol:** NDJSON over stdio. The Rust supervisor sends `call`
  messages, the Bun process replies with `db` / `stream` / `schedule` /
  `runFn` / `return` / `error` messages, all line-delimited JSON.
- **Concurrency model:** Single-threaded — every top-level function call
  serializes on `FnRunner::io_lock`. Bun is one process and the protocol
  isn't multiplexed at this layer.
- **Data access:** TypeScript code does **not** touch SQLite directly. Every
  `ctx.db.insert/get/query/...` is RPC'd back to Rust over stdio, executed
  against the Rust-owned `DataStore`, and the result is returned as a
  `db_result` message. Mutations hold the SQLite write lock for the entire
  duration of the handler.
- **Failure handling:** 30-second per-call timeout. On timeout the supervisor
  kills the Bun process and respawns it. The supervisor also respawns on any
  unexpected exit. A killed/respawned process loses all in-flight calls.
- **Why a subprocess and not embedded V8?** Two reasons: (1) Bun ships with
  TypeScript transformation, Node compat, and a fast cold start out of the
  box — embedding V8 means re-implementing all of that in Rust; (2) keeping
  the JS engine out-of-process means a buggy handler crashes only the Bun
  child, not the whole Pylon binary.

## How a function call flows

```
HTTP POST /api/fn/createOrder
        │
        ▼
┌──────────────────────────────┐
│  router (crates/router)      │  resolves /api/fn/* → fn dispatch
└──────────────────────────────┘
        │
        ▼
┌──────────────────────────────┐
│  FnRunner::call              │  acquires io_lock (serializes top-level calls)
│  (crates/functions/runner)   │  acquires SQLite write lock if mutation
└──────────────────────────────┘
        │
        ▼  CallMessage(call_id, fn_name, fn_type, args, auth)
┌──────────────────────────────┐
│  Bun child process           │  resolves the handler, awaits ctx.* calls
│  (packages/functions/        │
│   runtime.ts)                │
└──────────────────────────────┘
        │
        ▼  DbOpMessage{op: "insert", entity: "Order", data: {...}}
┌──────────────────────────────┐
│  FnRunner::recv loop         │  pulls one message, executes against DataStore,
│  execute_db_op               │  sends back DbResultMessage{call_id, op_id, data}
└──────────────────────────────┘
        │
        ▼  ReturnMessage(call_id, value)
┌──────────────────────────────┐
│  FnRunner::call returns      │  releases write lock, releases io_lock
└──────────────────────────────┘
```

Per call the cost is:
- **One spawn-time handshake** (paid once at boot, ~50-150ms on a hot disk).
- **One `call` message + one `return` message** to the child.
- **N round-trips** for `N` `ctx.db.*` operations the handler makes — each
  is a stdio write + JSON parse on both sides.

## What runs where

| Concern | Lives in | Notes |
|---|---|---|
| HTTP routing | `crates/router` | Platform-agnostic; reused on Workers target. |
| SQLite writes / reads | `crates/runtime` | Single writer, N reader connections. |
| WebSocket fanout | `crates/runtime/ws.rs` | 16 sharded broadcast channels. |
| Function dispatch | `crates/functions/runner.rs` | Owns the Bun process + protocol. |
| Function definitions | `packages/functions/runtime.ts` | Parses `functions/*.ts`, registers handlers. |
| User handler code | `packages/functions/runtime.ts` (inside Bun) | One Bun process loads everything. |
| Auth / policy | `crates/auth`, `crates/policy` | Run in Rust before / around fn dispatch. |

## Concurrency, in detail

There is exactly one Bun child per Pylon instance. Inside that child, calls
*can* run concurrently in JS (they're just promises) — the Rust side just
won't *send* a second `call` message until the first one returns, because
`io_lock` serializes the top-level dispatch.

That decision is deliberate:
- The protocol isn't multiplexed at the message layer (there's no per-call
  inbox demux on the Rust side).
- The single Bun event loop already serializes JS execution, so true
  parallelism inside the child wouldn't buy much.
- Mutations hold the SQLite write lock for the whole handler anyway — the
  bottleneck is writes, not JS execution.

**Reads (queries)** could parallelize across the read pool, but currently
don't — every fn call goes through `io_lock`. This is the single biggest
performance limit at the function layer today; see *Limits* below.

## Limits (read this before benchmarking)

- **Top-level calls serialize.** Even read-only `query` functions queue
  behind the call ahead. ~10K small-handler calls/sec on an M2 Mac;
  workloads dominated by complex handlers will be lower.
- **DB ops are stdio JSON.** Every `ctx.db.get(id)` is a stdio round-trip
  with two JSON encode/decode cycles. Cheap (~10µs) but it adds up — a
  handler that does 100 sequential `ctx.db.get()` calls eats ~1ms in
  protocol overhead before any actual work.
- **No per-call resource limits.** A handler can `while(true)` and the
  whole process burns CPU until the 30s timeout fires, killing every other
  in-flight call.
- **One bug = whole runtime down.** A handler that segfaults Bun, exhausts
  memory, or hits a Bun bug takes the entire Pylon function layer offline
  until the supervisor respawns (~100ms). In a single-tenant deployment
  this is fine. In a managed multi-tenant setup it's a noisy-neighbor
  problem.
- **Hot reload requires full process restart.** Editing one function
  invalidates Bun's module cache for everything; the supervisor restarts
  the whole runtime.
- **No sandboxing.** Handlers can read the filesystem, open sockets, exec
  subprocesses — whatever Bun lets them do. Fine for self-host, dangerous
  for multi-tenant.

## Alternatives we evaluated

This is honest, not a pitch. Each option is real and we considered it.

### 1. Stay with Bun subprocess (current)

- **Pros:** Working today. Bun's TS transform is fast and free. Real Node
  API compat means handlers can `import` from npm without ceremony. Single
  binary + `bun` on PATH is the entire dependency story.
- **Cons:** All of *Limits* above.
- **Verdict:** Right call for self-host and 1-instance-per-tenant
  deployments. Stop here unless you specifically need one of the things
  below.

### 2. Multiple Bun workers (subprocess pool)

- **Pros:** Cheap upgrade path. Spin up `N` Bun processes, route calls by
  `call_id % N`. Read queries parallelize across workers. Fault isolation
  improves — one worker crashing only drops its in-flight calls. ~1 day
  of work, no architectural change.
- **Cons:** Doesn't solve the SQLite single-writer bottleneck (mutations
  still queue). Worker pool needs a supervisor + load balancer that
  handles per-worker handshake state. Memory footprint scales with `N`
  (~80MB per Bun worker baseline).
- **Verdict:** First upgrade we'd take if the function layer becomes a
  bottleneck under read-heavy workloads. Plan to ship behind
  `PYLON_FN_WORKERS=N` with default `1`.

### 3. `deno_core` / `rusty_v8` — embedded V8 isolates

- **Pros:** True per-call isolates (each function call could get its own
  fresh JS context). Shared-memory DB ops (no JSON serialization — pass
  `serde_json::Value` directly across the FFI boundary). Cheap to spawn
  (microseconds vs Bun's ~50ms cold start). Memory limits per isolate.
  V8 snapshots make cold starts faster still.
- **Cons:** Embedding V8 is non-trivial (~2-3 weeks to get to parity with
  the current Bun-based runtime). Binary size grows by ~15MB. No automatic
  Node API compat — handlers can't `import` arbitrary npm packages without
  us writing polyfills or restricting to a curated stdlib. TypeScript
  transform needs SWC (separate dependency, ~5MB).
- **Verdict:** The right choice if and when we ship a managed
  multi-tenant cloud. The isolation story is what makes per-tenant
  sandboxing tractable. Not worth the rebuild for self-host.

### 4. `workerd` — Cloudflare's open-source Workers runtime

- **Pros:** Battle-tested isolate-per-request semantics. V8-based. Async
  by design. If we want first-class Cloudflare Workers parity, building
  on workerd locally means handlers behave identically in dev and on the
  Workers deploy target.
- **Cons:** Heavy (~120MB binary). Designed for HTTP-shaped workloads, not
  RPC-shaped — embedding it for our `ctx.db` round-trip pattern would be
  fighting the grain. Less flexible than `deno_core` for non-Workers
  targets.
- **Verdict:** Compelling specifically for the Workers target. If we end
  up shipping `pylon deploy --target workers` as a serious option, we
  should evaluate using workerd in dev too so handler behavior stays
  consistent. Not the right choice for the general runtime.

### 5. `deno_runtime` — full Deno as an embedded library

- **Pros:** Most complete embedded option. Sandboxed by default
  (permissions). TypeScript native. Top-tier Node API compat.
- **Cons:** ~50MB binary impact. Performance overhead vs Bun on
  short-handler workloads. Cold start on first call is heavier than
  necessary. Adds a large dependency surface.
- **Verdict:** No clear win over either Bun-subprocess (simpler) or
  `deno_core` (lighter). Skip.

### 6. `wasmtime` + JS-on-Wasm

- **Pros:** Wasm sandboxing is best-in-class. Could run handlers from
  untrusted sources safely.
- **Cons:** JS-on-Wasm is 5-20x slower than V8. Not viable for the
  perf characteristics Pylon needs.
- **Verdict:** Not a serious option today.

### 7. QuickJS / Boa — pure-Rust embedded JS

- **Pros:** Tiny binary impact. Fully in-process.
- **Cons:** Both are 10-50x slower than V8. No real Node compat. TypeScript
  needs an external transformer.
- **Verdict:** Reasonable for an "edge function" lite mode where size
  matters more than speed. Not a primary runtime.

## Recommendation

For the next 12 months: **stay on Bun subprocess.** It's working, it's
fast, and the limits don't bite at the workloads Pylon's positioned for
(self-host + per-tenant deploys + the cloud free tier).

Two things to add when the bottleneck becomes real:

1. **Worker pool** behind a flag. Defaults to 1 for backward-compat.
   Apps with read-heavy workloads opt in. ~1 day of work.

2. **`deno_core` runtime as a second option** when (and only when) we
   ship a true multi-tenant managed cloud where untrusted handler code
   needs sandboxing. ~3 weeks of work; ship it as
   `PYLON_FN_RUNTIME=isolates`, keep Bun as the default.

The wrong move would be to swap engines speculatively — every option above
has real costs and the demo workloads don't need any of them. Pick the
upgrade when the data justifies it.
