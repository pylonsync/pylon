/**
 * WebSocket fan-out harness.
 *
 * Spawn N persistent WebSocket clients, each subscribed to a live
 * query. A single writer pumps mutations into the entity those clients
 * are watching. Measures the round-trip from "write committed" to
 * "delta received by every subscriber" — the metric that tells you
 * how big a userbase one Pylon process can support.
 *
 * Run:
 *   bun benchmarks/ws-fanout/run.ts --subscribers 1000 --writers 1 --duration 30
 *
 * Notes:
 * - Subscribers connect to ws://localhost:4322 and send a `subscribe`
 *   frame matching the entity. Pylon sends them every change event
 *   matching the filter.
 * - The writer goes through HTTP /api/entities/* — same path a real
 *   client would take.
 * - Each subscriber timestamps received frames and reports the
 *   latency back to the parent on disconnect. We aggregate at the end.
 */

const args = parseArgs(process.argv.slice(2));
const SUBSCRIBERS = parseInt(args["subscribers"] ?? "1000", 10);
const WRITERS = parseInt(args["writers"] ?? "1", 10);
const DURATION_S = parseInt(args["duration"] ?? "30", 10);
const BASE_URL = args["base-url"] ?? "http://localhost:4321";
const WS_URL = args["ws-url"] ?? "ws://localhost:4322";
const ENTITY = args["entity"] ?? "Product";

console.log(
  `[fanout] subscribers=${SUBSCRIBERS} writers=${WRITERS} duration=${DURATION_S}s entity=${ENTITY}`,
);

// Mint a guest token to share. Real apps would have separate tokens
// per user but for the harness one token is fine.
const tokenRes = await fetch(`${BASE_URL}/api/auth/guest`, {
  method: "POST",
  headers: { Origin: BASE_URL },
});
if (!tokenRes.ok) throw new Error(`auth/guest failed: ${tokenRes.status}`);
const { token } = (await tokenRes.json()) as { token: string };

let established = 0;
let dead = 0;
const latencies: number[] = [];
const emittedAt = new Map<string, number>();

// ---- Subscribers ----
const sockets: WebSocket[] = [];
for (let i = 0; i < SUBSCRIBERS; i++) {
  const ws = new WebSocket(WS_URL);
  ws.addEventListener("open", () => {
    established++;
    ws.send(
      JSON.stringify({
        type: "auth",
        token,
      }),
    );
    ws.send(
      JSON.stringify({
        type: "subscribe",
        entity: ENTITY,
        filter: {},
      }),
    );
  });
  ws.addEventListener("message", (ev) => {
    let payload: unknown;
    try {
      payload = JSON.parse(typeof ev.data === "string" ? ev.data : "{}");
    } catch {
      return;
    }
    const id =
      (payload as { row_id?: string; id?: string }).row_id ??
      (payload as { id?: string }).id;
    if (!id) return;
    const sent = emittedAt.get(id);
    if (sent) latencies.push(performance.now() - sent);
  });
  ws.addEventListener("close", () => {
    dead++;
  });
  ws.addEventListener("error", () => {
    dead++;
  });
  sockets.push(ws);
  if (i % 200 === 0) await new Promise((r) => setTimeout(r, 5));
}

// Wait for connections to settle.
console.log(`[fanout] connecting…`);
let waited = 0;
while (established < SUBSCRIBERS && waited < 30_000) {
  await new Promise((r) => setTimeout(r, 250));
  waited += 250;
}
console.log(
  `[fanout] established=${established}/${SUBSCRIBERS} (after ${waited}ms)`,
);

// ---- Writer ----
let writeCount = 0;
const writerTask = async () => {
  const stop = Date.now() + DURATION_S * 1000;
  while (Date.now() < stop) {
    const id = `bench_${writeCount++}`;
    emittedAt.set(id, performance.now());
    const t0 = performance.now();
    await fetch(`${BASE_URL}/api/entities/${ENTITY}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Origin: BASE_URL,
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({
        // Minimal valid Product row for the store schema. Adjust if
        // you point this at a different entity.
        name: `bench-${id}`,
        description: "load test row",
        brand: "Bench",
        category: "Test",
        color: "gray",
        price: 1.0,
        rating: 0,
        stock: 0,
        createdAt: new Date().toISOString(),
      }),
    }).catch(() => {});
    // Throttle to a reasonable write rate: ~50 writes/sec across all
    // writers. Tweak if you want to push harder.
    await new Promise((r) => setTimeout(r, 1000 / (50 / WRITERS)));
  }
};

console.log(`[fanout] running writes for ${DURATION_S}s…`);
const writers = Array.from({ length: WRITERS }, writerTask);
await Promise.all(writers);

// Allow trailing deltas to land.
await new Promise((r) => setTimeout(r, 1000));

// ---- Report ----
const sorted = latencies.slice().sort((a, b) => a - b);
const pct = (p: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? 0;
console.log("");
console.log(`[fanout] subscribers=${SUBSCRIBERS}  writes=${writeCount}  deliveries=${latencies.length}`);
console.log(
  `[fanout]   p50=${pct(0.5).toFixed(1)}ms  p95=${pct(0.95).toFixed(1)}ms  p99=${pct(0.99).toFixed(1)}ms`,
);
console.log(`[fanout]   established=${established}  dead=${dead}`);
const expectedDeliveries = writeCount * SUBSCRIBERS;
const dropPct =
  expectedDeliveries > 0
    ? ((expectedDeliveries - latencies.length) / expectedDeliveries) * 100
    : 0;
console.log(
  `[fanout]   delivered=${((latencies.length / Math.max(1, expectedDeliveries)) * 100).toFixed(1)}%  dropped=${dropPct.toFixed(1)}%`,
);

for (const ws of sockets) {
  try {
    ws.close();
  } catch {}
}
process.exit(0);

function parseArgs(argv: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const next = argv[i + 1];
      if (next && !next.startsWith("--")) {
        out[key] = next;
        i++;
      } else {
        out[key] = "true";
      }
    }
  }
  return out;
}
