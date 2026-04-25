/**
 * Pylon Bench — in-browser load test dashboard.
 *
 * The main tab owns the "driver" role: it spawns WebWorker pools and
 * collects samples. Each worker hammers `bumpCounter` at a configured
 * rate. The dashboard aggregates samples into throughput + latency
 * percentiles per second and draws a live chart.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  configureClient,
  storageKey,
  callFn,
  db,
} from "@pylonsync/react";
import { Activity, Play, RefreshCw, Square } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@pylonsync/example-ui/card";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import { cn } from "@pylonsync/example-ui/utils";

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
  const [numWorkers, setNumWorkers] = useState(8);
  const [perWorkerRate, setPerWorkerRate] = useState(40);
  const [running, setRunning] = useState(false);
  const [totalMutations, setTotalMutations] = useState(0);
  const [totalFailures, setTotalFailures] = useState(0);
  const [series, setSeries] = useState<SecondBucket[]>([]);
  const [currentSecond, setCurrentSecond] = useState<SecondBucket>({
    sec: 0,
    count: 0,
    latencies: [],
  });

  const workersRef = useRef<Worker[]>([]);
  const runIdRef = useRef<string>("");

  const { data: counters } = db.useQuery<Counter>("Counter");

  useEffect(() => {
    ensureGuest().then(({ token }) => setToken(token));
  }, []);

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
    stop();
    runIdRef.current = `run_${Date.now().toString(36)}`;
    setTotalMutations(0);
    setTotalFailures(0);
    setSeries([]);
    setCurrentSecond({ sec: 0, count: 0, latencies: [] });

    for (let i = 0; i < numWorkers; i++) {
      const w = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
      w.onmessage = (e) => {
        const msg = e.data;
        if (msg.kind !== "sample") return;
        if (msg.ok) setTotalMutations((c) => c + 1);
        else setTotalFailures((c) => c + 1);
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
          label: `bench_${i % 16}`,
        },
      });
      workersRef.current.push(w);
    }
    setRunning(true);
  }, [token, numWorkers, perWorkerRate, stop]);

  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => {
      setCurrentSecond((cur) => {
        setSeries((prev) => {
          const next = [...prev, { ...cur, sec: prev.length }];
          return next.slice(-60);
        });
        return { sec: cur.sec + 1, count: 0, latencies: [] };
      });
    }, 1000);
    return () => clearInterval(t);
  }, [running]);

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
    try {
      await callFn("resetBench", {});
    } catch {}
  }

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
    const W = r.width;
    const H = r.height;
    ctx.clearRect(0, 0, W, H);

    ctx.strokeStyle = "rgba(255,255,255,0.08)";
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = (H / 4) * i;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(W, y);
      ctx.stroke();
    }

    if (series.length < 2) return;
    const n = series.length;
    const maxCount = Math.max(1, ...series.map((s) => s.count));
    const maxLat = Math.max(1, ...series.flatMap((s) => s.latencies), 100);

    const bw = W / 60;
    for (let i = 0; i < n; i++) {
      const b = series[i];
      const h = (b.count / maxCount) * H * 0.7;
      const x = i * bw;
      ctx.fillStyle = "rgba(139, 92, 246, 0.35)";
      ctx.fillRect(x, H - h, Math.max(1, bw - 1), h);
    }

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
    <div className="grid h-screen grid-rows-[56px_1fr]">
      <header className="flex items-center gap-6 border-b bg-background px-5">
        <div className="flex items-center gap-2.5 font-mono text-sm font-medium">
          <BrandMark />
          <span>Pylon · Bench</span>
        </div>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span
            className={cn(
              "inline-block size-2 rounded-full",
              running ? "animate-pulse bg-emerald-400" : "bg-muted",
            )}
          />
          {running ? "running" : "idle"}
          <Separator orientation="vertical" className="h-3" />
          <span>
            target <strong className="font-mono text-foreground">{targetTps}</strong> mut/s
          </span>
        </div>
      </header>

      <div className="grid grid-cols-[320px_1fr] gap-6 overflow-hidden p-6">
        <div className="flex flex-col gap-4 overflow-y-auto">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm uppercase tracking-wider text-muted-foreground">
                Config
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
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
              <p className="text-xs text-muted-foreground">
                Target = <strong className="font-mono text-foreground">{targetTps}</strong>{" "}
                mutations/sec across all workers.
              </p>

              <div className="flex gap-2">
                {!running ? (
                  <Button onClick={start} disabled={!token} className="flex-1">
                    <Play className="size-4" />
                    Start bench
                  </Button>
                ) : (
                  <Button variant="destructive" onClick={stop} className="flex-1">
                    <Square className="size-4" />
                    Stop
                  </Button>
                )}
                <Button variant="outline" onClick={reset} disabled={running}>
                  <RefreshCw className="size-4" />
                </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm uppercase tracking-wider text-muted-foreground">
                Hot rows
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-1.5">
              {(counters ?? []).slice(0, 16).map((c) => (
                <div
                  key={c.id}
                  className="flex items-center justify-between text-sm"
                >
                  <span className="font-mono text-xs text-muted-foreground">
                    {c.label}
                  </span>
                  <span className="font-mono tabular-nums">
                    {c.value.toLocaleString()}
                  </span>
                </div>
              ))}
              {(counters ?? []).length === 0 && (
                <div className="py-4 text-center text-xs text-muted-foreground">
                  Start a bench to populate rows.
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <div className="flex flex-col gap-4 overflow-hidden">
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 xl:grid-cols-7">
            <Metric label="TPS (live)" value={recentTps.toString()} sub="mut/s" />
            <Metric label="TPS (peak)" value={peakTps.toString()} sub="mut/s" />
            <Metric label="Total" value={totalMutations.toLocaleString()} />
            <Metric label="p50" value={p50.toString()} sub="ms" tone={p50 > 50 ? "warn" : "ok"} />
            <Metric label="p95" value={p95.toString()} sub="ms" tone={p95 > 200 ? "warn" : "ok"} />
            <Metric label="p99" value={p99.toString()} sub="ms" tone={p99 > 500 ? "warn" : "ok"} />
            <Metric label="Errors" value={totalFailures.toString()} tone={totalFailures > 0 ? "warn" : "ok"} />
          </div>

          <Card className="flex flex-1 flex-col overflow-hidden">
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="flex items-center gap-2 text-sm font-medium">
                <Activity className="size-4" />
                Throughput &amp; latency
              </CardTitle>
              <div className="flex items-center gap-3 text-xs text-muted-foreground">
                <Legend color="rgba(139, 92, 246, 0.5)" label="TPS" />
                <Legend color="#4ade80" label="p50" dashed />
                <Legend color="#8b5cf6" label="p95" />
                <span>· last 60s</span>
              </div>
            </CardHeader>
            <CardContent className="flex-1 p-0">
              <canvas ref={chartRef} className="h-full w-full" />
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}

function Knob({
  label,
  value,
  min,
  max,
  step,
  onChange,
  disabled,
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
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-xs">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-mono tabular-nums">{value}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        className="h-1 w-full cursor-pointer appearance-none rounded-full bg-muted accent-primary disabled:cursor-not-allowed disabled:opacity-50"
      />
    </div>
  );
}

function Metric({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: string;
  sub?: string;
  tone?: "ok" | "warn";
}) {
  return (
    <Card className={cn("p-3", tone === "warn" && "border-destructive/40")}>
      <div className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </div>
      <div className="mt-1 flex items-baseline gap-1 font-mono tabular-nums">
        <span className="text-xl font-semibold">{value}</span>
        {sub && <span className="text-xs text-muted-foreground">{sub}</span>}
      </div>
    </Card>
  );
}

function Legend({ color, label, dashed }: { color: string; label: string; dashed?: boolean }) {
  return (
    <span className="flex items-center gap-1.5">
      {dashed ? (
        <span
          className="block h-px w-3"
          style={{ borderTop: `1.5px dashed ${color}` }}
        />
      ) : (
        <span
          className="block h-2 w-3 rounded-sm"
          style={{ background: color }}
        />
      )}
      {label}
    </span>
  );
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="16" height="22" fill="currentColor" aria-hidden className="text-primary">
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}
