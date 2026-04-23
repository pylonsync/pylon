/**
 * Load-test worker. Runs inside a WebWorker so browser main-thread
 * scheduling doesn't bottleneck the measurements. Each worker posts
 * mutations at a target rate and reports per-mutation latency back
 * to the main thread via postMessage.
 */

type Config = {
  baseUrl: string;
  token: string;
  workerId: number;
  ratePerSec: number;      // how many mutations/sec THIS worker should aim for
  label: string;           // counter label this worker writes to
};

let cfg: Config | null = null;
let cancelled = false;

type Sample = { latencyMs: number; ok: boolean };

async function callBump(baseUrl: string, token: string, label: string): Promise<Sample> {
  const t0 = performance.now();
  try {
    const res = await fetch(`${baseUrl}/api/fn/bumpCounter`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ label, delta: 1 }),
    });
    if (!res.ok) {
      await res.text(); // drain
      return { latencyMs: performance.now() - t0, ok: false };
    }
    await res.json();
    return { latencyMs: performance.now() - t0, ok: true };
  } catch {
    return { latencyMs: performance.now() - t0, ok: false };
  }
}

async function run() {
  if (!cfg) return;
  const intervalMs = 1000 / cfg.ratePerSec;
  let next = performance.now();

  while (!cancelled) {
    const sample = await callBump(cfg.baseUrl, cfg.token, cfg.label);
    (self as unknown as Worker).postMessage({
      kind: "sample",
      workerId: cfg.workerId,
      ...sample,
    });
    next += intervalMs;
    const wait = next - performance.now();
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    else next = performance.now(); // fell behind — reset pacing
  }
  (self as unknown as Worker).postMessage({ kind: "stopped", workerId: cfg.workerId });
}

self.addEventListener("message", (e: MessageEvent) => {
  const msg = e.data;
  if (msg.kind === "start") {
    cfg = msg.config as Config;
    cancelled = false;
    run();
  } else if (msg.kind === "stop") {
    cancelled = true;
  }
});
