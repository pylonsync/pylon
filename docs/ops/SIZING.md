# Sizing

Measured on a 2024 laptop (Apple M-series, 16 GB RAM), in-memory SQLite.
Numbers are per-process throughput — SQLite is single-writer, so vertical
scaling of writes is bounded. Reads scale across a connection pool.

Re-run with:

```sh
cargo bench -p statecraft-runtime --bench bench
cargo bench -p statecraft-runtime --bench realtime_bench
```

## Data plane (single-writer SQLite)

| Operation                           | Ops/sec | Per op |
|---|---|---|
| `insert` (User, 3 fields)           | 68,000  | 14.6µs |
| `insert` (Todo, 4 fields)           | 77,000  | 13.0µs |
| `update`                            | 89,000  | 11.2µs |
| `delete` + reinsert                 | 40,000  | 24.7µs |
| `get_by_id`                         | 519,000 | 1.9µs  |
| `lookup` by unique field            | 484,000 | 2.1µs  |
| `query_filtered` (equality)         | 24,000  | 40.8µs |
| `query_filtered` ($like)            | 10,000  | 96.9µs |
| `list` (1000 rows)                  | 2,700   | 363µs  |
| `query_graph` (no filter, 1000 rows)| 1,500   | 660µs  |

## Realtime path

| Operation                           | Ops/sec | Per op |
|---|---|---|
| `change_log.append`                 | 5M      | 198ns  |
| `change_log.pull(100)`              | 85,000  | 11.7µs |
| `ws_hub.broadcast` (enqueue)        | 30,000  | 32.5µs |

The WS hub `broadcast` number is enqueue-side: it fans out to 16 shard
worker threads that each push to connected clients. Real delivery rate
depends on client count, message size, and TCP send buffers.

## What these numbers mean for deploy sizing

**Small (1 vCPU, 1 GB RAM, ~$5/mo VPS):**
- Up to ~20k writes/minute sustained, or bursts to 30k/minute
- Up to ~10k concurrent WS connections (64 KB stack per reader thread)
- Good for a few thousand active users at webapp levels of chattiness

**Medium (2 vCPU, 4 GB RAM, ~$25/mo VPS):**
- Up to ~50k writes/minute sustained
- Up to ~40k concurrent WS connections
- Good for 50k active users; room for complex queries without eviction

**Large (4+ vCPU, 8+ GB RAM):**
- Write ceiling is still single-writer SQLite (~70k inserts/sec peak).
  If you're pinned on writes, move to Postgres (`postgres-live` feature)
  or shard the app across databases.
- Reads scale with the read-connection pool. 4 pool connections × 500k
  reads/sec = 2M reads/sec ceiling.

## When to switch backends

**You're on SQLite and need Postgres when:**
- Sustained write rate > 50k/sec (you're at SQLite's single-writer limit)
- Multiple processes need to write (replicas, HA failover)
- You need online DDL / zero-downtime migrations at scale
- Storage > 100 GB (not a hard limit, but WAL checkpoints get painful)

**You can stay on SQLite when:**
- Single-process deployment
- Full DB fits comfortably in RAM for the read pool
- You back up with `statecraft backup` on a schedule

## What's NOT measured here

- Multi-client read contention (connection-pool fair-share)
- TLS handshake cost (reverse proxy terminates TLS)
- Network RTT — production numbers will be bounded by network first
- Shard tick budget for realtime game state — depends on `SimState::tick`

For a real capacity estimate under your workload, run `statecraft bench`
against a representative fixture and the manifest you'll ship with.
