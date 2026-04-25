/**
 * Catalog search load test.
 *
 * Hits POST /api/search/Product with random query + facet combinations
 * over a 10K-row catalog (the `examples/store` seed). Tracks request
 * rate and latency percentiles.
 *
 * Run: k6 run benchmarks/k6/catalog.js
 */
import http from "k6/http";
import { check } from "k6";
import { Rate, Trend } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "http://localhost:4321";

const errorRate = new Rate("errors");
const searchLatency = new Trend("search_latency_ms", true);

export const options = {
  scenarios: {
    ramp: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 50 },
        { duration: "1m", target: 200 },
        { duration: "2m", target: 500 },
        { duration: "30s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_duration: ["p(95)<200", "p(99)<500"],
    errors: ["rate<0.01"],
  },
};

const QUERIES = ["red", "shoes", "blue jacket", "atlas", "watch", ""];
const BRANDS = ["Atlas", "Orbit", "Nimbus", "Forge", "Quill"];
const CATEGORIES = ["Shoes", "Shirts", "Jackets", "Watches"];

let token = null;
export function setup() {
  const res = http.post(`${BASE_URL}/api/auth/guest`, null, {
    headers: { Origin: BASE_URL },
  });
  return { token: res.json("token") };
}

export default function (data) {
  const filters = {};
  if (Math.random() < 0.6) filters.brand = BRANDS[Math.floor(Math.random() * BRANDS.length)];
  if (Math.random() < 0.4) filters.category = CATEGORIES[Math.floor(Math.random() * CATEGORIES.length)];

  const body = {
    query: QUERIES[Math.floor(Math.random() * QUERIES.length)],
    filters,
    facets: ["brand", "category", "color"],
    page: Math.floor(Math.random() * 20),
    pageSize: 24,
  };

  const res = http.post(
    `${BASE_URL}/api/search/Product`,
    JSON.stringify(body),
    {
      headers: {
        "Content-Type": "application/json",
        Origin: BASE_URL,
        Authorization: `Bearer ${data.token}`,
      },
    },
  );

  const ok = check(res, {
    "200": (r) => r.status === 200,
    "has hits": (r) => r.json("hits") !== undefined,
  });
  if (!ok) errorRate.add(1);
  searchLatency.add(res.timings.duration);
}
