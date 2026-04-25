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

Reference workstation: M2 Pro, 32GB RAM, NVMe. Single Pylon process, SQLite default settings + the pragma tuning landed in this repo.

| Workload | Throughput | p95 |
|---|---|---|
| `entity_get_by_id` | 60K ops/sec | 0.02ms |
| `entity_insert` (single row) | 8K ops/sec | 0.15ms |
| `entity_insert` (txn batch of 100) | 200K rows/sec | — |
| `search` (10K rows, 3 facets) | 12K queries/sec | 0.4ms |
| `search` (1M rows, 3 facets) | 2K queries/sec | 1.5ms |
| WS fanout (1K subscribers, 100 writes/sec) | — | 4ms |
| WS fanout (10K subscribers, 100 writes/sec) | — | 35ms |

Subtract 30–50% if you're on a $5 VPS instead of an M2.

## Iterating on a perf change

1. Snapshot baseline: `cargo bench --bench bench > benchmarks/results/before.txt`
2. Make the change
3. Run again: `cargo bench --bench bench > benchmarks/results/after.txt`
4. Diff: `diff benchmarks/results/before.txt benchmarks/results/after.txt`

Criterion's HTML reports under `target/criterion/` are easier to read for percentile shifts.
