# Pylon benchmarks

Three lenses on Pylon performance. Run them on the binary you'd actually deploy to find where it falls apart.

## Quick start

```bash
# 1. Bring up Pylon under whichever app exercises the workload you care about.
cd examples/store && pylon dev      # search-heavy
cd examples/bench && pylon dev      # mutation-heavy
cd examples/arena && pylon dev      # WS-fanout-heavy

# 2. From the repo root, run a benchmark:
bun benchmarks/ws-fanout/run.ts --subscribers 5000 --writers 1
k6 run benchmarks/k6/catalog.js
cargo bench -p pylon-runtime --bench bench
```

## What each one measures

### `benchmarks/k6/`

HTTP-level load. k6 spins up `vus` (virtual users), each running the script's `default function` in a loop. Captures p50/p95/p99 latency and request rate. Use this to find the saturation point of a single Pylon process.

- `catalog.js` — search a 10K-product catalog with random filters
- `crud.js` — insert + read + update + delete cycle
- `mixed.js` — 80% read / 15% write / 5% search

```bash
brew install k6
k6 run benchmarks/k6/catalog.js
# Or with a custom URL:
k6 run -e BASE_URL=https://your-deploy.fly.dev benchmarks/k6/catalog.js
```

### `benchmarks/ws-fanout/`

WebSocket subscriber harness in Bun. Spawns N persistent WebSocket connections subscribed to live queries, then has a single writer thread pump M mutations/sec. Measures end-to-end RTT (write → server processes → client receives delta). The flow that breaks at 5K subscribers + 1K writes/sec on a single process.

```bash
bun install
bun benchmarks/ws-fanout/run.ts --subscribers 1000 --writers 1 --duration 30
```

Outputs:

```
subscribers=1000  writes=2978   p50=2ms   p95=12ms   p99=24ms
queue_drops=0     established=1000  dead=0
```

### `benchmarks/search/`

Rust criterion bench. Drives `Runtime::search` directly with no HTTP/WS overhead, so this measures pure storage + bitmap math. Tells you the ceiling — anything you see at the HTTP/WS layer is added by serialization + I/O.

```bash
cargo bench --manifest-path benchmarks/search/Cargo.toml
```

## Where the bottlenecks actually live

| Symptom in production | Run this to confirm |
|---|---|
| API p99 climbs above 100ms during writes | `k6 run mixed.js`, watch SQLite WAL fsync via `strace` |
| Live queries lag during a hot write loop | `ws-fanout/run.ts` with mutator + N=10000 |
| Search is fast standalone but slow under concurrent writes | `cargo bench --bench bench` *while* `mixed.js` runs |
| Memory grows during long-running sessions | Run `k6 run mixed.js --duration 1h`, watch `pylon` RSS |

## Reading the numbers

Measured on an M2 Pro, 32GB RAM, NVMe. Single Pylon process, in-memory
SQLite, post-tuning (the pragma + `prepare_cached` work that lives
upstream of these benches).

### Search (`benchmarks/search`)

```
=== 10K rows ===
  empty query, page 0                  363µs/op    2,754 ops/sec
  text 'red'                           624µs/op    1,602 ops/sec
  filter brand+category                351µs/op    2,850 ops/sec
  sort price asc, page 5               7.57ms/op     132 ops/sec

=== 100K rows ===
  empty query, page 0                  2.61ms/op     383 ops/sec
  text 'red'                           5.57ms/op     179 ops/sec
  filter brand                         2.69ms/op     371 ops/sec
```

Notes from the run:
- Filter + facet queries scale near-linearly with matching set size,
  not table size. 100K rows is only ~7× slower than 10K because the
  bitmap intersection touches the same number of bits.
- Text search is dominated by FTS5 token scoring; "red" matches ~half
  the catalog so the BM25 ranking step does real work.
- Sorted pagination hits a known bottleneck: the planner currently
  materializes every hit into a temp table for `ORDER BY price ASC`,
  which collapses to 132 ops/sec. There's a planned optimization to
  push the sort down to an index when `sort` matches a sortable
  column.

### What you should expect

Subtract 30–50% on a $5 VPS. Add roughly 1ms per request for HTTP
overhead, 2ms for an authenticated REST call (policy eval + JSON
serialize/deserialize). WebSocket-delivered live-query updates skip
both.

## Iterating on a perf change

1. Snapshot baseline: `cargo bench --bench bench > benchmarks/results/before.txt`
2. Make the change
3. Run again: `cargo bench --bench bench > benchmarks/results/after.txt`
4. Diff: `diff benchmarks/results/before.txt benchmarks/results/after.txt`

Criterion's HTML reports under `target/criterion/` are easier to read for percentile shifts.
