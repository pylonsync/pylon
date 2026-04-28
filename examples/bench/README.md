# Bench — in-browser load test

A self-contained load test for Pylon. Spawns N virtual client
WebWorkers, each hammering `bumpCounter` at a configured rate. The
main tab aggregates samples and shows live throughput + latency
percentiles. No external tooling — go break it in your browser.

**What this example demonstrates:**

- **Honest throughput measurement.** Workers run in separate threads
  so the browser's main-thread scheduling doesn't bottleneck reads
  of `performance.now()`. Each mutation is round-tripped through a
  real WebSocket; latency is a real end-to-end time.
- **Percentiles, not averages.** p50/p95/p99 per-second and over the
  whole run. Averages hide the long tail; percentiles don't.
- **Hot-row contention is visible.** Workers rotate across 16 labels,
  so `bump_0` through `bump_15` see concurrent writes. Watch the
  "Hot rows" panel to see the effect of contention on specific rows.
- **Runs are replayable.** Every 1-second bucket is written to the
  `Sample` table with the `runId`. Query the log afterward to compare
  configurations.

## Run

```bash
cd examples/bench
bun install
bun run dev          # starts Pylon server on :4321

# in a second terminal
cd web
bun install
bun run dev          # serves the UI on :5176
```

Open <http://localhost:5176>. Set the virtual client count + per-
client rate. Click **Start bench**.

## Reading the dashboard

| Metric | Meaning |
|---|---|
| **TPS (live)** | Mutations completed in the last second |
| **TPS (peak)** | Highest 1-second bucket in the current run |
| **Total** | Successful mutations since run started |
| **p50 / p95 / p99** | End-to-end mutation latency percentiles over the whole run |
| **Errors** | Non-2xx responses or network failures |

The chart shows the last 60s: violet bars = TPS, violet line = p95
latency, green dashed = p50 latency.

## Recommended configs

- **Baseline** — 8 clients × 40 mut/sec = 320 TPS target. This is
  what a laptop should serve with p95 < 10ms on a local Pylon.
- **Stretch** — 32 clients × 100 mut/sec = 3200 TPS target. Expect
  p50 still <10ms if the storage backend is SQLite; p95 will climb
  under IO pressure.
- **Break it** — 128 clients × 200 mut/sec = 25.6K TPS target.
  Storage will be the bottleneck; latency percentiles will flat-top
  and errors will appear when WS write queues fill.

## Files

- `app.ts` — `Counter` + `Sample` entities
- `functions/bumpCounter.ts` — upsert + increment (the workload)
- `functions/logSample.ts` — per-second bucket sink
- `functions/resetBench.ts` — wipe counters + samples
- `client/BenchApp.tsx` — dashboard: knobs, metrics strip, live chart
- `client/worker.ts` — WebWorker that drives `bumpCounter` at a
  configured rate and reports per-call latency back
- `web/` — Vite UI mounting `BenchApp`

## Run it

The point of the bench is to measure your hardware, your workload, your network. Sample a few configurations:

- **Smoke test**: 4 clients × 25 mut/sec. Latency should be a few milliseconds.
- **Mid-load**: 16 clients × 50 mut/sec. Watch p95 stay flat and TPS scale linearly.
- **Saturation**: bump clients × rate until p99 visibly climbs and TPS stops growing. That's your single-process ceiling for this workload.

For higher write throughput than SQLite's single-writer ceiling, switch to Postgres mode (`DATABASE_URL=postgres://...`) and re-run — the bench is identical; only the backend changes.

For canonical numbers across hardware tiers, see [Sizing](https://docs.pylonsync.com/operations/sizing).
