# Sync tier on Durable Objects — design

Status: Option C's client relay shipped in v0.4.24. The same `PylonSync` DO now
also provides the managed origin-to-origin `ClusterBus` transport for Pylon
Cloud. The implementation includes `POST /sync/cluster/push` and
`GET /sync/cluster/ws`, a durable replay ring, stable publish IDs, duplicate
removal, and per-project derived keys.

The client relay building blocks are the `PylonSync` DO
(`crates/workers/src/{sync_do,relay_core}.rs`), `ChangeLogSink` +
machine push (`crates/runtime/src/sync_relay.rs`), signed auth blob
(`crates/auth/src/relay_blob.rs`), `GET /api/sync/relay-token`, and
`relay: true` client mode (TS + Swift). Setup docs:
apps/docs/operations/sync-relay.mdx. Still open: a deployed wrangler
integration test, the live dual-write diff on cloud traffic, and the
autostop flip (steps 2–5 below — cloud-side rollout, not framework).

## Managed cluster extension

The client relay and the cluster transport use separate rings and socket tags.
Client sockets receive policy-filtered `ChangeEvent` frames. Origin sockets
receive trusted raw `ClusterBus` envelopes. These envelopes include changes,
presence, session changes, and CRDT snapshots.

The control plane derives `HMAC(root, "pylon-relay-app-v1\0" + projectId)`.
It injects only this derived value into customer machines. The relay Worker
keeps the root key and derives the same project key before it verifies a
request. A customer app cannot use its key to publish to another project.

The runtime opens the origin WebSocket before it serves traffic. A configured
relay failure stops startup. The publisher retries with the same message ID
and applies backpressure instead of dropping a committed event. The DO removes
duplicate retries and assigns a relay sequence for reconnect replay.

## Why

`apps/control-plane/lib/fly/machineConfig.ts:79` sets `autostop = input.autostop ?? false`
with `min_machines_running: 1`. Every paid project runs a Fly machine 24 hours a day
whether or not anyone connects. `meterFlyUsage.ts` confirms it: "paid tier,
autostop=false."

Idle floor per project: shared-1x at 1 GB is $2.02 base + ~$3.75 additional RAM +
$0.45 volume ≈ **$6.20/mo doing nothing**. Starter is $25/mo with
`maxProjectsPerOrg: Number.MAX_SAFE_INTEGER`. Four idle projects and that customer
is underwater before any egress.

Autostop is off because the machine terminates sync WebSockets. A machine holding
live connections never goes idle, so it can never stop. The stateful tier pins the
stateless tier to always-on.

Durable Objects with WebSocket hibernation break that coupling, and it is the one
thing Fly has no answer for. Move sync off the machine and the machine becomes
ordinary request-serving compute that can autostop.

## What owns the change log

`ChangeEvent.seq` (`crates/sync/src/lib.rs`) is a single global monotonic counter,
allocated by `crates/runtime/src/seq_allocator.rs` against a persisted high-water
mark in `_pylon_change_seq`. Single-writer by construction. Whoever owns `seq`
owns the write path.

### Option A — DO owns data, change log, and seq

Rejected. DO storage becomes the database. Loses Postgres, loses volume-snapshot
backup (`backupProject` / `createVolumeFromSnapshot`), loses the runtime's mutation
pipeline. This is the "Pylon runs entirely on Workers" story, which is a separate
and much larger bet.

### Option B — DO owns change log + seq, machine owns row data

Rejected. Two writers, one truth. The mutation pipeline writes rows inside a
SQLite/PG transaction and appends to the change log as part of the same logical
operation. Split across a network boundary, a mutation can commit rows and fail to
append. No transaction spans both. Fixing that means an outbox, which is Option C
with extra steps.

### Option C — machine owns change log + seq, DO is a filtered fan-out relay ✅

- `seq_allocator` stays exactly where it is. No protocol change, no new
  `410 RESYNC_REQUIRED` class.
- The mutation pipeline is untouched.
- After commit, the machine pushes the `ChangeEvent` to the app's DO.
- The DO holds hibernating sockets and fans out.
- The DO owns no truth. It owns delivery.

## What the DO has to do that nothing does today

### 1. Per-subscriber policy filtering

This is the hard part. Not the wasm plumbing.

`crates/runtime/src/ws.rs` filters every change event per client through
`PolicyEngine` against an `AuthContext` enriched with active-org roles
(`AuthEnricher`). A DO that fans out unfiltered leaks every row to every client.
A DO that filters without enriched roles denies every frame for any policy calling
`auth.hasAnyRole(...)` — the exact 0.4.4 bug, reintroduced on a new substrate.

**Approach: enrich once at handshake, on the machine.** The machine already
authenticates the WS upgrade (`WsAuth` + `AuthEnricher`). It mints a signed,
enriched auth blob and hands it to the DO. The DO caches it for the connection's
lifetime and runs `pylon-policy` against it locally. The DO never touches the
database.

`pylon-policy` is already a direct dependency of `pylon-workers`
(`crates/workers/Cargo.toml:34`), so it is inside the `ci.yml:230` wasm32 check
and compiles to `wasm32-unknown-unknown` today.

Cost of this approach: staleness. A role revoked mid-connection keeps its old
grants until reconnect. Mitigate with a TTL on the blob that forces periodic
re-handshake. An explicit invalidation push from the machine on role change is the
follow-up, not the first cut.

### 2. Catch-up reads

A reconnecting client presents a cursor. Today that is `pull_range(since, limit)`
against the machine's disk log, which wakes the machine and defeats the purpose.

`ChangeLogStore` already splits this two ways. Keep the split, move one side:

- `load_recent(limit)` — the recent ring. Moves to DO storage, served by the DO.
  Machine stays asleep. Covers the common reconnect.
- `pull_range(since, limit)` — deep history. Stays on the machine. A cursor older
  than the DO's ring wakes the machine, which is correct and rare.

No new concept; this mirrors the trait's existing shape.

### 3. Wake-on-write

A client sending a mutation over the socket needs a running machine. Fly autostart
handles it when the DO calls the machine over HTTP. Cold-start latency is
acceptable on a write and avoided entirely on reads the DO serves from its ring.

## Async refactor scope

Smaller than `crates/workers/README.md` implies. Option C needs almost none of it.

- The DO class is already fully async. `rooms_do.rs:88-288` — `fetch`,
  `websocket_message`, `websocket_close`, `hydrate_from_storage`, and every handler.
  No `block_on` inside it.
- All ~30 `block_on` sites live in the **Worker-side adapters**: `d1_store`,
  `kv_cache`, `r2_files`, `queue_jobs`, `handler`, and the `WorkersRooms` /
  `pubsub` `RoomOps` impls. Those exist to run Pylon *on* Workers. Option C uses
  none of them.
- Therefore `DataStore` stays sync. `RoomOps` stays sync. No blast radius across
  every platform.

Note that `block_on` on `wasm32-unknown-unknown` cannot yield to the JS event loop,
so a pending promise never resolves. Those call sites compile and then hang on first
use. `cargo check` cannot catch it, which is why CI is green on code that would
deadlock. The comment at `rooms_do.rs:27` claiming the single-threaded runtime
tolerates this is wrong. That refactor stays on the shelf for the full Workers
story; it is not on this critical path.

What Option C actually needs built:

1. A `PylonSync` DO class alongside `PylonRoom`: hibernating sockets, recent-event
   ring in DO storage, per-subscriber policy filtering.
2. A `ChangeLogSink` on the machine — a post-commit hook pushing committed events
   to the DO. Fire-and-forget with retry; the disk log stays the source of truth,
   so a dropped push degrades to a catch-up read, not data loss.
3. Signed auth-blob mint (machine) and verify (DO).

## Sequencing

1. `PylonSync` DO + a **live** integration test: open a socket, force hibernation,
   assert a fanned-out event arrives correctly filtered. A deployed wrangler test,
   not `cargo check`.
2. `ChangeLogSink` behind a flag, dual-write: the machine fans out as it does today
   **and** pushes to the DO. Diff the two on live traffic.
3. Cut WS clients over to the DO once the diff is clean. Machine keeps serving HTTP.
4. Flip the autostop default to true for paid projects.
5. Measure the idle-machine line on the next Fly invoice.

## The risk worth naming

Step 4 is where the money is. Step 1 is where the uncertainty is. If the DO cannot
reproduce the per-subscriber filter exactly, this ships a data leak rather than a
cost saving. The dual-write in step 2 exists so that correctness is proven against
live traffic before anything is trusted.
