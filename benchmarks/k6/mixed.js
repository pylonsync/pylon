/**
 * Realistic mixed workload — 80% reads, 15% writes, 5% search.
 *
 * Models the kind of traffic a real B2B app sees: lots of dashboards
 * polling entity lists, occasional CRUD, search bursts when users
 * navigate.
 */
import http from "k6/http";
import { check } from "k6";

const BASE_URL = __ENV.BASE_URL || "http://localhost:4321";

export const options = {
  scenarios: {
    sustained: {
      executor: "constant-vus",
      vus: 200,
      duration: __ENV.DURATION || "5m",
    },
  },
  thresholds: {
    http_req_failed: ["rate<0.01"],
    http_req_duration: ["p(95)<150", "p(99)<400"],
  },
};

export function setup() {
  const res = http.post(`${BASE_URL}/api/auth/guest`, null, {
    headers: { Origin: BASE_URL },
  });
  return { token: res.json("token") };
}

export default function (data) {
  const headers = {
    "Content-Type": "application/json",
    Origin: BASE_URL,
    Authorization: `Bearer ${data.token}`,
  };

  const r = Math.random();
  if (r < 0.8) {
    // Read: list a random entity
    const entity = ["Product", "User", "CartItem"][Math.floor(Math.random() * 3)];
    const res = http.get(`${BASE_URL}/api/entities/${entity}?limit=24`, { headers });
    check(res, { "read ok": (r) => r.status === 200 || r.status === 403 });
  } else if (r < 0.95) {
    // Write: insert a CartItem
    http.post(
      `${BASE_URL}/api/entities/CartItem`,
      JSON.stringify({
        userId: "bench",
        productId: `prod_${__ITER}`,
        productName: "Bench item",
        productBrand: "Atlas",
        productPrice: 9.99,
        quantity: 1,
        addedAt: new Date().toISOString(),
      }),
      { headers },
    );
  } else {
    // Search
    http.post(
      `${BASE_URL}/api/search/Product`,
      JSON.stringify({ query: "red", page: 0, pageSize: 24 }),
      { headers },
    );
  }
}
