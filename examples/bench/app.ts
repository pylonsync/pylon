/**
 * Pylon Bench — in-browser load test.
 *
 * The UI spawns N virtual client WebWorkers. Each worker maintains its
 * own WebSocket to the Pylon server and hammers either mutations or
 * query subscriptions at a configurable rate. A live dashboard in the
 * main tab collects per-worker samples and renders:
 *
 *   - throughput (mutations/sec, events/sec)
 *   - latency percentiles (p50, p95, p99)
 *   - connection count, CPU-bound client samples
 *   - hot entity row counts
 *
 * Why bother: "realtime backends" are all trivial at 10 users. The
 * interesting question is how they degrade under load. This demo
 * lets you reproducibly measure that in under a minute.
 */
import { entity, field, policy, buildManifest } from "@pylonsync/sdk";

// A single tiny entity the workers write to. Kept intentionally small
// so IO cost is dominated by the sync/broadcast path, not the storage.
const Counter = entity(
  "Counter",
  {
    label: field.string(),
    value: field.int(),
    updatedAt: field.datetime(),
  },
  {
    indexes: [
      { name: "by_label", fields: ["label"], unique: true },
    ],
  },
);

// Per-second bucketed aggregate of the run. Written by the main tab;
// read by the HUD. Separate from the counter so we can bench pure
// inserts without contention on a single row.
const Sample = entity(
  "Sample",
  {
    runId: field.string(),
    atSec: field.int(),
    mutations: field.int(),
    p50ms: field.float(),
    p95ms: field.float(),
    p99ms: field.float(),
  },
  {
    indexes: [
      { name: "by_run_at", fields: ["runId", "atSec"], unique: true },
    ],
  },
);

const counterPolicy = policy({
  name: "counter_public",
  entity: "Counter",
  allowRead: "true",
  allowInsert: "auth.userId != null",
  allowUpdate: "auth.userId != null",
});

const samplePolicy = policy({
  name: "sample_public",
  entity: "Sample",
  allowRead: "true",
  allowInsert: "auth.userId != null",
});

const manifest = buildManifest({
  name: "bench",
  version: "0.1.0",
  entities: [Counter, Sample],
  queries: [],
  actions: [],
  policies: [counterPolicy, samplePolicy],
  routes: [],
});

console.log(JSON.stringify(manifest, null, 2));
