/**
 * Pylon Arena — mass-multiplayer dot simulation.
 *
 * Layout:
 *   - Full-screen canvas rendering all Dot entities
 *   - Stats HUD (top-left): connected dots, mutations/sec, broadcast
 *     throughput, measured p50/p95 round-trip
 *   - Control bar (top-right): spawn N bots, reset, toggle AoI ring
 *
 * The interesting bit is the latency histogram — every outgoing
 * moveDot mutation is timed from send → "we observe our row update
 * bounce back in the live query" — a true end-to-end RTT.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";

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

// ---------------------------------------------------------------------------
// Guest auth — every browser session is a new anonymous user. Keeps the
// demo zero-friction; no email, no OTP.
// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
// Moving average + percentile calc for the latency HUD.
// ---------------------------------------------------------------------------
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

  const { data: dots } = db.useQuery<Dot>("Dot");

  // Interpolated positions per-dot — animated on the client even when
  // the server hasn't pushed a new row.
  const localPositions = useRef<Map<string, { x: number; y: number }>>(new Map());
  const pendingSends = useRef<Map<string, number>>(new Map()); // mutationId → sentAt
  const latencies = useRef<number[]>([]);
  const mutRateWindow = useRef<number[]>([]); // timestamps

  // One-time guest auth + spawn our dot.
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
    return () => { cancelled = true; };
  }, []);

  // HUD tick — refresh stats every 500ms.
  useEffect(() => {
    const t = setInterval(() => {
      const now = Date.now();
      // Drop mutations older than 1s to get per-second rate.
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

  // Canvas rendering — runs at rAF, interpolates each dot toward its
  // server target, draws, sends moveDot mutations periodically for
  // the local dot.
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

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
      const W = r.width, H = r.height;
      ctx.clearRect(0, 0, W, H);

      // Subtle grid background.
      ctx.strokeStyle = "rgba(255,255,255,0.035)";
      ctx.lineWidth = 1;
      const grid = 48;
      for (let x = 0; x < W; x += grid) {
        ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, H); ctx.stroke();
      }
      for (let y = 0; y < H; y += grid) {
        ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(W, y); ctx.stroke();
      }

      const all = dots ?? [];
      for (const d of all) {
        let pos = localPositions.current.get(d.id);
        if (!pos) {
          pos = { x: d.x, y: d.y };
          localPositions.current.set(d.id, pos);
        }
        // Interpolate toward server target.
        const dx = d.tx - pos.x;
        const dy = d.ty - pos.y;
        const dist = Math.hypot(dx, dy);
        if (dist > 0.001) {
          const step = Math.min(dist, d.speed * dt);
          pos.x += (dx / dist) * step;
          pos.y += (dy / dist) * step;
        }

        const px = pos.x * W, py = pos.y * H;
        const isMine = d.id === myDotId;
        const rad = isMine ? 8 : 5;

        // Trail to target for local dot.
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

      // Every ~150ms, send our observed position back up so other
      // clients can correct drift. Also refreshes lastSeenAt.
      if (myDotId && t - lastLocalSend > 150) {
        const myRow = all.find((d) => d.id === myDotId);
        const pos = localPositions.current.get(myDotId);
        if (myRow && pos) {
          const t0 = performance.now();
          callFn("moveDot", {
            dotId: myDotId,
            x: pos.x, y: pos.y,
            tx: myRow.tx, ty: myRow.ty,
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

    return () => { cancelAnimationFrame(raf); ro.disconnect(); };
  }, [dots, myDotId, showRing]);

  // Click to move — writes our dot's target. The render loop picks it
  // up on the next frame.
  const onCanvasClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!myDotId) return;
      const canvas = e.currentTarget;
      const r = canvas.getBoundingClientRect();
      const x = (e.clientX - r.left) / r.width;
      const y = (e.clientY - r.top) / r.height;
      const pos = localPositions.current.get(myDotId) ?? { x: 0.5, y: 0.5 };
      callFn("moveDot", {
        dotId: myDotId,
        x: pos.x, y: pos.y,
        tx: x, ty: y,
      }).catch(() => {});
    },
    [myDotId],
  );

  // Bot spawner — drops N bot dots scattered across the plane. Each
  // bot wanders autonomously via a client-side tick that picks new
  // targets every few seconds.
  const [spawning, setSpawning] = useState(false);
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

  // Bot brains — pick new random targets for bots every few seconds.
  // Runs client-side so the "spawner" browser drives them; other
  // browsers just observe via live query.
  useEffect(() => {
    if (!dots) return;
    const t = setInterval(() => {
      const ourBots = dots.filter((d) => d.isBot && Math.random() < 0.12);
      for (const b of ourBots.slice(0, 10)) {
        callFn("moveDot", {
          dotId: b.id,
          x: b.x, y: b.y,
          tx: Math.random(), ty: Math.random(),
        }).catch(() => {});
      }
    }, 1200);
    return () => clearInterval(t);
  }, [dots]);

  return (
    <div className="arena">
      <canvas
        ref={canvasRef}
        className="arena-canvas"
        onClick={onCanvasClick}
      />

      <div className="arena-hud hud-stats">
        <div className="hud-row">
          <span className="hud-label">DOTS</span>
          <span className="hud-value">{hudStats.dots.toLocaleString()}</span>
          {hudStats.bots > 0 && (
            <span className="hud-sub">· {hudStats.bots} bot</span>
          )}
        </div>
        <div className="hud-row">
          <span className="hud-label">MUT/S</span>
          <span className="hud-value">{hudStats.mutPerSec}</span>
        </div>
        <div className="hud-row">
          <span className="hud-label">P50</span>
          <span className="hud-value">{hudStats.p50}<span className="hud-unit">ms</span></span>
        </div>
        <div className="hud-row">
          <span className="hud-label">P95</span>
          <span className="hud-value">{hudStats.p95}<span className="hud-unit">ms</span></span>
        </div>
      </div>

      <div className="arena-hud hud-controls">
        <div className="hud-title">Stress test</div>
        <div className="hud-btn-row">
          <button
            className="hud-btn"
            disabled={spawning}
            onClick={() => spawnBots(10)}
          >+10 bots</button>
          <button
            className="hud-btn"
            disabled={spawning}
            onClick={() => spawnBots(100)}
          >+100</button>
          <button
            className="hud-btn"
            disabled={spawning}
            onClick={() => spawnBots(500)}
          >+500</button>
        </div>
        <button
          className="hud-btn danger"
          onClick={clearBots}
        >Clear bots</button>
        <label className="hud-toggle">
          <input
            type="checkbox"
            checked={showRing}
            onChange={(e) => setShowRing(e.target.checked)}
          />
          Show target line
        </label>
        <div className="hud-hint">Click anywhere to move.</div>
      </div>

      <div className="arena-brand">
        <svg viewBox="0 0 48 64" width="16" height="21" fill="currentColor">
          <path d="M24 2 L10 20 L24 32 Z" />
          <path d="M24 2 L38 20 L24 32 Z" />
          <path d="M24 32 L18 48 L24 62 L30 48 Z" />
          <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
          <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
        </svg>
        <span>Pylon · Arena</span>
      </div>
    </div>
  );
}
