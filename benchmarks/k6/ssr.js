/**
 * SSR render saturation test (open model) with status-code accounting.
 *
 * Fixed REQUEST RATE (not fixed VUs) is the right model for finding where
 * SSR falls apart: each render is a Rust→Bun round-trip, so once arrival
 * rate exceeds render throughput, concurrency (≈ rate × latency) climbs,
 * trips the dispatch caps (PYLON_HTTP_INFLIGHT_MAX / _PER_IP_MAX → 503),
 * and p99 explodes. Sweep RATE and watch status_503 / status_err appear.
 *
 *   k6 run -e RATE=400 -e DURATION=15s benchmarks/k6/ssr.js
 *   k6 run -e RATE=2000 -e PATH=/product/x benchmarks/k6/ssr.js   # dynamic vs cached
 */
import http from "k6/http";
import { Counter } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "http://localhost:4500";
const TARGET = __ENV.ROUTE || "/";
const RATE = parseInt(__ENV.RATE || "400", 10);
const DURATION = __ENV.DURATION || "15s";

const okCount = new Counter("ok");          // 200
const capRejects = new Counter("capReject"); // 503 (dispatch cap shed)
const connFails = new Counter("connFail");   // status 0: reset/timeout/refused

export const options = {
  discardResponseBodies: true,
  scenarios: {
    fixed: {
      executor: "constant-arrival-rate",
      rate: RATE,
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: Math.min(RATE * 2, 6000),
      maxVUs: Math.min(RATE * 6, 12000),
    },
  },
};

export function setup() {
  // Sequential warmup only — a concurrent batch trips the per-IP cap and
  // hangs setup(). The scenario itself keeps the Bun pool warm after ~1s.
  for (let i = 0; i < 10; i++) http.get(`${BASE_URL}${TARGET}`);
}

export default function () {
  const res = http.get(`${BASE_URL}${TARGET}`);
  if (res.status === 200) okCount.add(1);
  else if (res.status === 503) capRejects.add(1);
  else connFails.add(1);
}
