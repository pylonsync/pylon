/**
 * SSR render THROUGHPUT (closed model, keep-alive).
 *
 * A bounded pool of VUs (= persistent keep-alive connections) hammers a
 * route back-to-back. Achieved req/s = VUS / render_latency, so as VUS
 * climbs, rps rises until the server saturates (latency climbs, rps
 * plateaus). The plateau is the true SSR render ceiling — measured with
 * a handful of connections instead of a connection flood, so it's light
 * on the load generator and isolates render throughput from the per-IP
 * dispatch cap (raise PYLON_HTTP_PER_IP_MAX on the server to match VUS).
 *
 *   k6 run -e VUS=64 -e DURATION=8s benchmarks/k6/ssr-throughput.js
 */
import http from "k6/http";
import { Counter } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "http://localhost:4500";
const ROUTE = __ENV.ROUTE || "/"; // NOT "PATH" — that collides with $PATH
const VUS = parseInt(__ENV.VUS || "64", 10);
const DURATION = __ENV.DURATION || "8s";

const ok = new Counter("ok");
const bad = new Counter("bad"); // non-200 (429/503/0/etc.)

export const options = {
  discardResponseBodies: true,
  scenarios: {
    closed: { executor: "constant-vus", vus: VUS, duration: DURATION },
  },
};

export default function () {
  const res = http.get(`${BASE_URL}${ROUTE}`);
  if (res.status === 200) ok.add(1);
  else bad.add(1);
}
