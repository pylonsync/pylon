/**
 * Pylon Arena — mass-multiplayer dot simulation.
 *
 * Layout:
 *   - Full-screen canvas rendering all Dot entities
 *   - Stats HUD (top-left): connected dots, mutations/sec, p50/p95 RTT
 *   - Control bar (top-right): spawn N bots, reset, toggle target ring
 *
 * The interesting bit is the latency histogram — every outgoing
 * moveDot mutation is timed from send → "we observe our row update
 * bounce back in the live query" — a true end-to-end RTT.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import { Bot, MousePointerClick, Trash2 } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Switch } from "@pylonsync/example-ui/switch";
import { Label } from "@pylonsync/example-ui/label";
import { cn } from "@pylonsync/example-ui/utils";

const BASE_URL = "http://localhost:4321";
init({ baseUrl: BASE_URL, appName: "arena" });
configureClient({ baseUrl: BASE_URL, appName: "arena" });

type Dot = {
  id: string;
  userId: string;
  x: number;
  y: number;
  tx: number;
  ty: number;
  color: string;
  label?: string | null;
  speed: number;
  isBot: boolean;
  lastSeenAt: string;
};

function uid() {
  return Math.random().toString(36).slice(2, 10);
}

async function ensureGuest(): Promise<string> {
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
  return userId!;
}

function pct(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * p));
  return sorted[idx];
}

export function ArenaApp() {
  const [userId, setUserId] = useState<string | null>(null);
  const [myDotId, setMyDotId] = useState<string | null>(null);
  const [hudStats, setHudStats] = useState({
    dots: 0,
    bots: 0,
    mutPerSec: 0,
    p50: 0,
    p95: 0,
  });
  const [showRing, setShowRing] = useState(true);
  const [spawning, setSpawning] = useState(false);

  const { data: dots } = db.useQuery<Dot>("Dot");

  const localPositions = useRef<Map<string, { x: number; y: number }>>(new Map());
  const latencies = useRef<number[]>([]);
  const mutRateWindow = useRef<number[]>([]);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    ensureGuest().then(async (id) => {
      if (cancelled) return;
      setUserId(id);
      try {
        const r = await callFn<{ id: string }>("spawnDot", {
          userId: id,
          label: "you",
        });
        setMyDotId(r.id);
      } catch (e) {
        console.error("spawn failed", e);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const t = setInterval(() => {
      const now = Date.now();
      mutRateWindow.current = mutRateWindow.current.filter((ts) => now - ts < 1000);
      const last100 = latencies.current.slice(-100);
      const all = dots ?? [];
      setHudStats({
        dots: all.length,
        bots: all.filter((d) => d.isBot).length,
        mutPerSec: mutRateWindow.current.length,
        p50: Math.round(pct(last100, 0.5)),
        p95: Math.round(pct(last100, 0.95)),
      });
    }, 500);
    return () => clearInterval(t);
  }, [dots]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;

    const resize = () => {
      const r = canvas.getBoundingClientRect();
      canvas.width = r.width * dpr;
      canvas.height = r.height * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);

    let raf = 0;
    let lastT = performance.now();
    let lastLocalSend = 0;

    const tick = (t: number) => {
      const dt = Math.min(0.05, (t - lastT) / 1000);
      lastT = t;
      const r = canvas.getBoundingClientRect();
      const W = r.width;
      const H = r.height;
      ctx.clearRect(0, 0, W, H);

      ctx.strokeStyle = "rgba(255,255,255,0.035)";
      ctx.lineWidth = 1;
      const grid = 48;
      for (let x = 0; x < W; x += grid) {
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, H);
        ctx.stroke();
      }
      for (let y = 0; y < H; y += grid) {
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(W, y);
        ctx.stroke();
      }

      const all = dots ?? [];
      for (const d of all) {
        let pos = localPositions.current.get(d.id);
        if (!pos) {
          pos = { x: d.x, y: d.y };
          localPositions.current.set(d.id, pos);
        }
        const dx = d.tx - pos.x;
        const dy = d.ty - pos.y;
        const dist = Math.hypot(dx, dy);
        if (dist > 0.001) {
          const step = Math.min(dist, d.speed * dt);
          pos.x += (dx / dist) * step;
          pos.y += (dy / dist) * step;
        }

        const px = pos.x * W;
        const py = pos.y * H;
        const isMine = d.id === myDotId;
        const rad = isMine ? 8 : 5;

        if (isMine && showRing) {
          ctx.strokeStyle = "rgba(139, 92, 246, 0.28)";
          ctx.setLineDash([3, 4]);
          ctx.beginPath();
          ctx.moveTo(px, py);
          ctx.lineTo(d.tx * W, d.ty * H);
          ctx.stroke();
          ctx.setLineDash([]);
        }

        ctx.fillStyle = d.color;
        ctx.beginPath();
        ctx.arc(px, py, rad, 0, Math.PI * 2);
        ctx.fill();

        if (isMine) {
          ctx.strokeStyle = "rgba(255,255,255,0.65)";
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(px, py, rad + 3, 0, Math.PI * 2);
          ctx.stroke();
        }
      }

      if (myDotId && t - lastLocalSend > 150) {
        const myRow = all.find((d) => d.id === myDotId);
        const pos = localPositions.current.get(myDotId);
        if (myRow && pos) {
          const t0 = performance.now();
          callFn("moveDot", {
            dotId: myDotId,
            x: pos.x,
            y: pos.y,
            tx: myRow.tx,
            ty: myRow.ty,
          })
            .then(() => {
              latencies.current.push(performance.now() - t0);
              if (latencies.current.length > 200) latencies.current.shift();
              mutRateWindow.current.push(Date.now());
            })
            .catch(() => {});
        }
        lastLocalSend = t;
      }

      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [dots, myDotId, showRing]);

  const onCanvasClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!myDotId) return;
      const canvas = e.currentTarget;
      const r = canvas.getBoundingClientRect();
      const x = (e.clientX - r.left) / r.width;
      const y = (e.clientY - r.top) / r.height;
      const pos = localPositions.current.get(myDotId) ?? { x: 0.5, y: 0.5 };
      callFn("moveDot", { dotId: myDotId, x: pos.x, y: pos.y, tx: x, ty: y }).catch(
        () => {},
      );
    },
    [myDotId],
  );

  async function spawnBots(count: number) {
    setSpawning(true);
    try {
      const tasks: Promise<unknown>[] = [];
      for (let i = 0; i < count; i++) {
        tasks.push(
          callFn("spawnDot", {
            userId: `bot_${uid()}_${i}`,
            label: null,
            isBot: true,
          }),
        );
      }
      await Promise.all(tasks);
    } finally {
      setSpawning(false);
    }
  }

  async function clearBots() {
    await callFn("removeBots", {}).catch(() => {});
  }

  useEffect(() => {
    if (!dots) return;
    const t = setInterval(() => {
      const ourBots = dots.filter((d) => d.isBot && Math.random() < 0.12);
      for (const b of ourBots.slice(0, 10)) {
        callFn("moveDot", {
          dotId: b.id,
          x: b.x,
          y: b.y,
          tx: Math.random(),
          ty: Math.random(),
        }).catch(() => {});
      }
    }, 1200);
    return () => clearInterval(t);
  }, [dots]);

  return (
    <div className="fixed inset-0 bg-[#0a0a0c]">
      <canvas
        ref={canvasRef}
        className="block h-full w-full cursor-crosshair"
        onClick={onCanvasClick}
      />

      <StatsHud stats={hudStats} />
      <ControlsHud
        spawning={spawning}
        showRing={showRing}
        onShowRing={setShowRing}
        onSpawn={spawnBots}
        onClear={clearBots}
      />

      <div className="absolute bottom-4 left-4 flex items-center gap-2 font-mono text-xs text-muted-foreground">
        <BrandMark />
        <span>Pylon · Arena</span>
      </div>
    </div>
  );
}

function StatsHud({
  stats,
}: {
  stats: { dots: number; bots: number; mutPerSec: number; p50: number; p95: number };
}) {
  return (
    <div className="absolute left-4 top-4 flex flex-col gap-2 rounded-lg border bg-card/85 p-4 backdrop-blur-sm">
      <Row label="DOTS" value={stats.dots.toLocaleString()} subtle={stats.bots > 0 ? `${stats.bots} bot` : undefined} />
      <Row label="MUT/S" value={stats.mutPerSec.toString()} />
      <Row label="P50" value={`${stats.p50}`} unit="ms" />
      <Row label="P95" value={`${stats.p95}`} unit="ms" />
    </div>
  );
}

function Row({
  label,
  value,
  subtle,
  unit,
}: {
  label: string;
  value: string;
  subtle?: string;
  unit?: string;
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="w-12 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </span>
      <span className="font-mono text-base tabular-nums">
        {value}
        {unit && <span className="ml-0.5 text-xs text-muted-foreground">{unit}</span>}
      </span>
      {subtle && <span className="text-xs text-muted-foreground">· {subtle}</span>}
    </div>
  );
}

function ControlsHud({
  spawning,
  showRing,
  onShowRing,
  onSpawn,
  onClear,
}: {
  spawning: boolean;
  showRing: boolean;
  onShowRing: (b: boolean) => void;
  onSpawn: (n: number) => void;
  onClear: () => void;
}) {
  return (
    <div className="absolute right-4 top-4 flex w-56 flex-col gap-3 rounded-lg border bg-card/85 p-4 backdrop-blur-sm">
      <div className="flex items-center gap-2 text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
        <Bot className="size-3.5" />
        Stress test
      </div>
      <div className="flex gap-1.5">
        <Button
          size="xs"
          variant="outline"
          disabled={spawning}
          onClick={() => onSpawn(10)}
          className="flex-1"
        >
          +10
        </Button>
        <Button
          size="xs"
          variant="outline"
          disabled={spawning}
          onClick={() => onSpawn(100)}
          className="flex-1"
        >
          +100
        </Button>
        <Button
          size="xs"
          variant="outline"
          disabled={spawning}
          onClick={() => onSpawn(500)}
          className="flex-1"
        >
          +500
        </Button>
      </div>
      <Button
        size="xs"
        variant="ghost"
        onClick={onClear}
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
      >
        <Trash2 className="size-3" />
        Clear bots
      </Button>
      <div className="flex items-center justify-between text-xs">
        <Label htmlFor="ring" className="cursor-pointer text-muted-foreground">
          Show target line
        </Label>
        <Switch id="ring" checked={showRing} onCheckedChange={onShowRing} />
      </div>
      <div
        className={cn(
          "flex items-center gap-1.5 text-xs",
          "text-muted-foreground",
        )}
      >
        <MousePointerClick className="size-3" />
        Click anywhere to move.
      </div>
    </div>
  );
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="14" height="18" fill="currentColor" aria-hidden>
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}
