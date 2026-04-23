/**
 * Pylon Bench — in-browser load test dashboard.
 *
 * The main tab owns the "driver" role: it spawns WebWorker pools and
 * collects samples. Each worker hammers `bumpCounter` at a configured
 * rate. The dashboard aggregates samples into throughput + latency
 * percentiles per second and draws a live chart.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  configureClient,
  storageKey,
  callFn,
  db,
} from "@pylonsync/react";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "bench" });
configureClient({ baseUrl: BASE_URL, appName: "bench" });

type Counter = {
  id: string;
  label: string;
  value: number;
  updatedAt: string;
};

async function ensureGuest(): Promise<{ token: string; userId: string }> {
  let token = localStorage.getItem(storageKey("token"));
  let userId = localStorage.getItem(storageKey("user"));
  if (!token || !userId) {
    const res = await fetch(`${BASE_URL}/api/auth/guest`, { method: "POST" });
    const body = await res.json();
    token = body.token as string;
    userId = body.user_id as string;
    localStorage.setItem(storageKey("token"), token);
    localStorage.setItem(storageKey("user"), userId);
  }
  return { token: token!, userId: userId! };
}

function pct(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

type SecondBucket = {
  sec: number;
  count: number;
  latencies: number[];
};

export function BenchApp() {
  const [token, setToken] = useState<string | null>(null);

  // Config knobs.
  const [numWorkers, setNumWorkers] = useState(8);
  const [perWorkerRate, setPerWorkerRate] = useState(40); // mutations/sec
  const [running, setRunning] = useState(false);

  // Live metrics.
  const [totalMutations, setTotalMutations] = useState(0);
  const [totalFailures, setTotalFailures] = useState(0);
  const [series, setSeries] = useState<SecondBucket[]>([]);
  const [currentSecond, setCurrentSecond] = useState<SecondBucket>({
    sec: 0, count: 0, latencies: [],
  });

  const workersRef = useRef<Worker[]>([]);
  const runIdRef = useRef<string>("");
  const secondStartRef = useRef<number>(0);

  // Live count of Counter rows — shows the actual server-side effect.
  const { data: counters } = db.useQuery<Counter>("Counter");

  // Auth on mount.
  useEffect(() => {
    ensureGuest().then(({ token }) => setToken(token));
  }, []);

  // ---- Bench lifecycle ----

  const stop = useCallback(() => {
    for (const w of workersRef.current) {
      w.postMessage({ kind: "stop" });
      setTimeout(() => w.terminate(), 200);
    }
    workersRef.current = [];
    setRunning(false);
  }, []);

  const start = useCallback(() => {
    if (!token) return;
    stop(); // clear any prior run
    runIdRef.current = `run_${Date.now().toString(36)}`;
    secondStartRef.current = Math.floor(performance.now() / 1000);
    setTotalMutations(0);
    setTotalFailures(0);
    setSeries([]);
    setCurrentSecond({ sec: 0, count: 0, latencies: [] });

    for (let i = 0; i < numWorkers; i++) {
      const w = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
      w.onmessage = (e) => {
        const msg = e.data;
        if (msg.kind !== "sample") return;
        if (msg.ok) {
          setTotalMutations((c) => c + 1);
        } else {
          setTotalFailures((c) => c + 1);
        }
        setCurrentSecond((s) => ({
          sec: s.sec,
          count: s.count + 1,
          latencies: [...s.latencies, msg.latencyMs],
        }));
      };
      w.postMessage({
        kind: "start",
        config: {
          baseUrl: BASE_URL,
          token,
          workerId: i,
          ratePerSec: perWorkerRate,
          label: `bench_${i % 16}`, // spread across 16 hot rows
        },
      });
      workersRef.current.push(w);
    }
    setRunning(true);
  }, [token, numWorkers, perWorkerRate, stop]);

  // Flush current second into series once per second.
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => {
      setCurrentSecond((cur) => {
        // Roll into series.
        setSeries((prev) => {
          const next = [...prev, { ...cur, sec: prev.length }];
          return next.slice(-60); // keep last 60 seconds
        });
        return { sec: cur.sec + 1, count: 0, latencies: [] };
      });
    }, 1000);
    return () => clearInterval(t);
  }, [running]);

  // Upload each bucket as a Sample row so runs can be compared later.
  useEffect(() => {
    if (series.length === 0) return;
    const last = series[series.length - 1];
    callFn("logSample", {
      runId: runIdRef.current,
      atSec: last.sec,
      mutations: last.count,
      p50ms: Math.round(pct(last.latencies, 0.5)),
      p95ms: Math.round(pct(last.latencies, 0.95)),
      p99ms: Math.round(pct(last.latencies, 0.99)),
    }).catch(() => {});
  }, [series]);

  const all = useMemo(() => {
    const flat: number[] = [];
    for (const b of series) flat.push(...b.latencies);
    return flat;
  }, [series]);

  const p50 = Math.round(pct(all, 0.5));
  const p95 = Math.round(pct(all, 0.95));
  const p99 = Math.round(pct(all, 0.99));
  const peakTps = series.reduce((m, b) => Math.max(m, b.count), 0);
  const recentTps = series.length > 0 ? series[series.length - 1].count : 0;
  const targetTps = numWorkers * perWorkerRate;

  async function reset() {
    stop();
    try { await callFn("resetBench", {}); } catch {}
  }

  // Chart rendering.
  const chartRef = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    const c = chartRef.current;
    if (!c) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const r = c.getBoundingClientRect();
    c.width = r.width * dpr;
    c.height = r.height * dpr;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const W = r.width, H = r.height;
    ctx.clearRect(0, 0, W, H);

    // Axes.
    ctx.strokeStyle = "#262626";
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = (H / 4) * i;
      ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(W, y); ctx.stroke();
    }

    if (series.length < 2) return;
    const n = series.length;
    const maxCount = Math.max(1, ...series.map((s) => s.count));
    const maxLat = Math.max(1, ...series.flatMap((s) => s.latencies), 100);

    // TPS bars.
    const bw = W / 60;
    for (let i = 0; i < n; i++) {
      const b = series[i];
      const h = (b.count / maxCount) * H * 0.7;
      const x = i * bw;
      ctx.fillStyle = "rgba(139, 92, 246, 0.35)";
      ctx.fillRect(x, H - h, Math.max(1, bw - 1), h);
    }

    // p95 line.
    ctx.strokeStyle = "#8b5cf6";
    ctx.lineWidth = 2;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const b = series[i];
      const p = pct(b.latencies, 0.95);
      const y = H - (p / maxLat) * H;
      const x = i * bw + bw / 2;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // p50 line.
    ctx.strokeStyle = "#4ade80";
    ctx.lineWidth = 1.5;
    ctx.setLineDash([3, 4]);
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const b = series[i];
      const p = pct(b.latencies, 0.5);
      const y = H - (p / maxLat) * H;
      const x = i * bw + bw / 2;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    ctx.setLineDash([]);
  }, [series]);

  return (
    <div className="bn-app">
      <div className="bn-topbar">
        <div className="bn-brand">
          <svg viewBox="0 0 48 64" width="18" height="24" fill="currentColor">
            <path d="M24 2 L10 20 L24 32 Z" />
            <path d="M24 2 L38 20 L24 32 Z" />
            <path d="M24 32 L18 48 L24 62 L30 48 Z" />
            <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
            <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
          </svg>
          <span>Pylon · Bench</span>
        </div>
        <div className="bn-status">
          <span className={`bn-dot ${running ? "on" : ""}`} />
          {running ? "running" : "idle"}
          <span className="bn-sep">·</span>
          target <b>{targetTps}</b> mut/s
        </div>
      </div>

      <div className="bn-body">
        <div className="bn-left">
          <div className="bn-section">
            <div className="bn-section-title">Config</div>
            <Knob
              label="Virtual clients"
              value={numWorkers}
              min={1}
              max={128}
              step={1}
              onChange={setNumWorkers}
              disabled={running}
            />
            <Knob
              label="Per-client mut/sec"
              value={perWorkerRate}
              min={1}
              max={200}
              step={1}
              onChange={setPerWorkerRate}
              disabled={running}
            />
            <div className="bn-hint">
              Target = <b>{numWorkers * perWorkerRate}</b> mutations/sec across all workers.
            </div>

            <div className="bn-btn-row">
              {!running ? (
                <button className="bn-btn primary" onClick={start} disabled={!token}>
                  Start bench
                </button>
              ) : (
                <button className="bn-btn danger" onClick={stop}>
                  Stop
                </button>
              )}
              <button className="bn-btn" onClick={reset} disabled={running}>
                Reset
              </button>
            </div>
          </div>

          <div className="bn-section">
            <div className="bn-section-title">Hot rows</div>
            <div className="bn-counters">
              {(counters ?? []).slice(0, 16).map((c) => (
                <div key={c.id} className="bn-counter">
                  <span className="bn-counter-label">{c.label}</span>
                  <span className="bn-counter-value">{c.value.toLocaleString()}</span>
                </div>
              ))}
              {(counters ?? []).length === 0 && (
                <div className="bn-empty">Start a bench to populate rows.</div>
              )}
            </div>
          </div>
        </div>

        <div className="bn-right">
          <div className="bn-metrics">
            <Metric label="TPS (live)" value={recentTps.toString()} sub="mut/s" />
            <Metric label="TPS (peak)" value={peakTps.toString()} sub="mut/s" />
            <Metric label="Total" value={totalMutations.toLocaleString()} />
            <Metric label="p50" value={p50.toString()} sub="ms" tone={p50 > 50 ? "warn" : "ok"} />
            <Metric label="p95" value={p95.toString()} sub="ms" tone={p95 > 200 ? "warn" : "ok"} />
            <Metric label="p99" value={p99.toString()} sub="ms" tone={p99 > 500 ? "warn" : "ok"} />
            <Metric label="Errors" value={totalFailures.toString()} tone={totalFailures > 0 ? "warn" : "ok"} />
          </div>

          <div className="bn-chart-wrap">
            <div className="bn-chart-legend">
              <span><i className="bn-legend-bar" /> TPS</span>
              <span><i className="bn-legend-line line-p50" /> p50</span>
              <span><i className="bn-legend-line line-p95" /> p95</span>
              <span className="bn-legend-win">· last 60s</span>
            </div>
            <canvas ref={chartRef} className="bn-chart" />
          </div>
        </div>
      </div>
    </div>
  );
}

function Knob({
  label, value, min, max, step, onChange, disabled,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  return (
    <div className="bn-knob">
      <div className="bn-knob-head">
        <span className="bn-knob-label">{label}</span>
        <span className="bn-knob-value">{value}</span>
      </div>
      <input
        type="range"
        className="bn-knob-range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

function Metric({
  label, value, sub, tone,
}: {
  label: string;
  value: string;
  sub?: string;
  tone?: "ok" | "warn";
}) {
  return (
    <div className={`bn-metric ${tone ?? ""}`}>
      <div className="bn-metric-label">{label}</div>
      <div className="bn-metric-value">
        {value}
        {sub && <span className="bn-metric-sub">{sub}</span>}
      </div>
    </div>
  );
}
