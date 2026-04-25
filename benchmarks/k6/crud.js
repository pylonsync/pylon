/**
 * CRUD-cycle load test.
 *
 * Each VU loops: insert → fetch → update → delete on the User entity.
 * Captures p95 of each phase separately so you can see if writes
 * are the bottleneck (they usually are).
 */
import http from "k6/http";
import { check } from "k6";
import { Trend } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "http://localhost:4321";

const insertT = new Trend("insert_ms", true);
const getT = new Trend("get_ms", true);
const updateT = new Trend("update_ms", true);
const deleteT = new Trend("delete_ms", true);

export const options = {
  scenarios: {
    ramp: {
      executor: "ramping-vus",
      startVUs: 1,
      stages: [
        { duration: "20s", target: 20 },
        { duration: "1m", target: 100 },
        { duration: "20s", target: 0 },
      ],
    },
  },
  thresholds: {
    http_req_failed: ["rate<0.01"],
    insert_ms: ["p(95)<50"],
    get_ms: ["p(95)<10"],
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

  // Insert
  const ins = http.post(
    `${BASE_URL}/api/entities/User`,
    JSON.stringify({
      email: `bench-${__VU}-${__ITER}@example.com`,
      displayName: `Bench User ${__VU}-${__ITER}`,
      avatarColor: "#8b5cf6",
      createdAt: new Date().toISOString(),
    }),
    { headers },
  );
  insertT.add(ins.timings.duration);
  if (!check(ins, { "insert ok": (r) => r.status === 200 || r.status === 201 })) return;
  const id = ins.json("id");

  // Get
  const got = http.get(`${BASE_URL}/api/entities/User/${id}`, { headers });
  getT.add(got.timings.duration);
  check(got, { "get ok": (r) => r.status === 200 });

  // Update
  const upd = http.patch(
    `${BASE_URL}/api/entities/User/${id}`,
    JSON.stringify({ displayName: "Updated" }),
    { headers },
  );
  updateT.add(upd.timings.duration);

  // Delete
  const del = http.del(`${BASE_URL}/api/entities/User/${id}`, null, { headers });
  deleteT.add(del.timings.duration);
}
