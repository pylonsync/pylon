import React, { useCallback, useEffect, useRef, useState } from "react";
import { callFn, db, getSync, init, storageKey, useRoom } from "@pylonsync/react";
import type { FireEvent, Game, GameStats } from "../game/game";
import type { AvatarRow } from "../game/remote";

/**
 * World3D — multiplayer procedural-island FPS on Pylon SSR.
 *
 * The page SSRs as a lightweight shell (instant first paint), then a
 * client-only effect dynamic-imports the game engine — three.js never
 * loads during SSR and ships as its own async chunk. Pylon live
 * queries (<SyncBridge/>) feed avatar poses and destruction state
 * into the engine through a narrow setter API.
 */

interface DestructionRow {
  id: string;
  key: string;
  userId: string;
  createdAt: string;
}

async function newGuestSession(): Promise<string> {
  const res = await fetch("/api/auth/guest", { method: "POST" });
  if (!res.ok) throw new Error(`guest auth failed: ${res.status}`);
  const body = (await res.json()) as { token: string; user_id: string };
  window.localStorage.setItem(storageKey("token"), body.token);
  window.localStorage.setItem(storageKey("userId"), body.user_id);
  window.localStorage.setItem(storageKey("isGuest"), "1");
  return body.user_id;
}

async function ensureGuestSession(): Promise<string> {
  const token = window.localStorage.getItem(storageKey("token"));
  const userId = window.localStorage.getItem(storageKey("userId"));
  if (token && userId) {
    // A stored token can be stale (dev server restarted, session
    // expired). Probe it — and note /api/auth/session returns 200
    // with `user_id: null` for unknown tokens, so check the BODY,
    // not the status.
    try {
      const res = await fetch("/api/auth/session", {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.ok) {
        const body = (await res.json()) as { session?: { user_id?: string | null } };
        if (body.session?.user_id) return userId;
      }
    } catch {
      // Network blip — fall through to a fresh session.
    }
    window.localStorage.removeItem(storageKey("token"));
    window.localStorage.removeItem(storageKey("userId"));
  }
  return newGuestSession();
}

/**
 * Client-only child: subscribes to the live queries and pushes updates
 * into the game. Mounted only after the game engine is ready, so the
 * sync engine never spins up during SSR/hydration.
 */
function SyncBridge({ game, userId }: { game: Game; userId: string }) {
  const { data: avatars } = db.useQuery<AvatarRow>("Avatar");
  const { data: destruction } = db.useQuery<DestructionRow>("Destruction");

  // Transient fire events (tracers, grenades in flight) ride the
  // rooms WS relay — no DB writes, fire-and-forget. Durable state
  // (poses, destruction) stays on entities above.
  const { broadcast } = useRoom("battle", userId);

  useEffect(() => {
    if (avatars) game.setAvatars(avatars, userId);
  }, [game, avatars, userId]);

  useEffect(() => {
    if (destruction) game.syncDestruction(new Set(destruction.map((d) => d.key)));
  }, [game, destruction]);

  useEffect(() => {
    game.onLocalFire((ev) => broadcast("fire", ev));
    const off = getSync().subscribeRoomMessages("battle", (message) => {
      // Own broadcasts echo back — peers only.
      if (message.topic !== "fire" || message.from === userId) return;
      game.applyRemoteFire(message.payload as FireEvent, message.from);
    });
    return () => {
      game.onLocalFire(() => {});
      off();
    };
  }, [game, userId, broadcast]);

  return null;
}

export default function IslandPage() {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const gameRef = useRef<Game | null>(null);
  const [game, setGame] = useState<Game | null>(null);
  const [userId, setUserId] = useState<string | null>(null);
  const [locked, setLocked] = useState(false);
  // Full menu only before the first deploy; afterwards a lost pointer
  // lock (alt-tab, Esc, focus loss) shows a minimal resume pill instead.
  const [hasDeployed, setHasDeployed] = useState(false);
  const [stats, setStats] = useState<GameStats | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);

  // Boot: dynamic-import the engine, auth, spawn, wire callbacks.
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    let disposed = false;

    // Same-origin client: without init(), callFn falls back to the
    // hard-coded dev default (localhost:4321) and breaks on any other
    // port or in production. init() resolves window.location.origin.
    init({ appName: "world3d" });

    (async () => {
      const { Game } = await import("../game/game");
      if (disposed) return;
      const g = new Game(mount);
      gameRef.current = g;
      // Dev console handle — lets you (or an agent) poke the live
      // scene: __world3d.game, .three (the module), etc.
      (window as unknown as Record<string, unknown>).__world3d = { game: g };
      g.onStats(setStats);
      g.start();
      setGame(g);

      // Auth + spawn with retry — `pylon dev` restarts the server for
      // a beat on every file save, and a page that loads mid-restart
      // shouldn't be stuck on "Failed to fetch".
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          const uid = await ensureGuestSession();
          if (disposed) return;
          setUserId(uid);
          const spawn = g.spawnPoint;
          const r = await callFn<{ id: string }>("spawnAvatar", {
            userId: uid,
            x: spawn.x,
            y: spawn.y,
            z: spawn.z,
          });
          if (!disposed) {
            g.setAvatarId(r.id);
            setBootError(null);
          }
          break;
        } catch (err) {
          console.error(`[world3d] session boot failed (attempt ${attempt + 1})`, err);
          if (disposed) return;
          if (attempt === 4) setBootError(String(err));
          else await new Promise((resolve) => setTimeout(resolve, 800 * (attempt + 1)));
        }
      }
    })().catch((err) => {
      console.error("[world3d] engine boot failed", err);
      if (!disposed) setBootError(String(err));
    });

    const onLockChange = () => {
      const isLocked = document.pointerLockElement !== null;
      setLocked(isLocked);
      if (isLocked) setHasDeployed(true);
    };
    document.addEventListener("pointerlockchange", onLockChange);

    return () => {
      disposed = true;
      document.removeEventListener("pointerlockchange", onLockChange);
      gameRef.current?.dispose();
      gameRef.current = null;
    };
  }, []);

  const enterGame = useCallback(() => gameRef.current?.requestPointerLock(), []);
  const resetIsland = useCallback(() => {
    callFn("resetIsland", {}).catch(() => {});
  }, []);

  return (
    <div className="fixed inset-0">
      <div ref={mountRef} className="absolute inset-0" />

      {/* Crosshair */}
      {locked && (
        <div className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2">
          <div className="relative h-5 w-5 opacity-80">
            <span className="absolute left-1/2 top-0 h-1.5 w-px -translate-x-1/2 bg-white" />
            <span className="absolute bottom-0 left-1/2 h-1.5 w-px -translate-x-1/2 bg-white" />
            <span className="absolute left-0 top-1/2 h-px w-1.5 -translate-y-1/2 bg-white" />
            <span className="absolute right-0 top-1/2 h-px w-1.5 -translate-y-1/2 bg-white" />
          </div>
        </div>
      )}

      {/* Top-left: perf + sync HUD */}
      <div className="absolute left-4 top-4 select-none rounded-lg bg-black/45 px-4 py-3 font-mono text-xs leading-5 text-zinc-200 backdrop-blur-sm">
        <div className="mb-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-400">
          Pylon · World3D
        </div>
        {stats ? (
          <>
            <HudRow label="fps" value={`${stats.fps}`} />
            <HudRow label="draws" value={`${stats.drawCalls}`} />
            <HudRow label="tris" value={`${(stats.triangles / 1e6).toFixed(2)}M`} />
            <HudRow label="players" value={`${stats.players}`} />
            <HudRow label="sync" value={`${stats.mutPerSec}/s · p95 ${stats.p95}ms`} />
            <HudRow
              label="ruins"
              value={`${stats.blocksDestroyed}/${stats.blocksTotal}`}
            />
          </>
        ) : (
          <div className="text-zinc-400">booting island…</div>
        )}
      </div>

      {/* Bottom-center: ammo + state */}
      {locked && stats && (
        <div className="pointer-events-none absolute bottom-6 left-1/2 -translate-x-1/2 select-none text-center">
          <div className="font-mono text-2xl font-semibold tracking-widest text-white/90">
            {stats.reloading ? "RELOADING" : `${stats.ammo} / 30`}
          </div>
          {stats.swimming && (
            <div className="mt-1 text-xs uppercase tracking-[0.3em] text-cyan-200/80">
              swimming
            </div>
          )}
        </div>
      )}

      {/* Resume pill: lock lost mid-game (alt-tab, Esc, focus loss) */}
      {!locked && hasDeployed && (
        <button
          type="button"
          onClick={enterGame}
          className="absolute inset-0 flex cursor-pointer items-end justify-center pb-16"
        >
          <span className="rounded-full bg-black/60 px-5 py-2 font-mono text-sm text-zinc-100">
            paused — click to resume
          </span>
        </button>
      )}

      {/* First-run deploy menu */}
      {!locked && !hasDeployed && (
        <button
          type="button"
          onClick={enterGame}
          className="absolute inset-0 flex cursor-pointer flex-col items-center justify-center bg-black/35 text-center backdrop-blur-[2px]"
        >
          <div className="text-3xl font-bold tracking-tight text-white drop-shadow">
            {game ? "Click to deploy" : "Generating island…"}
          </div>
          <div className="mt-4 grid grid-cols-2 gap-x-8 gap-y-1 font-mono text-sm text-zinc-200/90">
            <span className="text-right text-zinc-400">WASD</span><span className="text-left">move</span>
            <span className="text-right text-zinc-400">shift</span><span className="text-left">sprint</span>
            <span className="text-right text-zinc-400">space</span><span className="text-left">jump / swim up</span>
            <span className="text-right text-zinc-400">mouse</span><span className="text-left">aim · fire</span>
            <span className="text-right text-zinc-400">G / right-click</span><span className="text-left">grenade</span>
            <span className="text-right text-zinc-400">R</span><span className="text-left">reload</span>
            <span className="text-right text-zinc-400">V</span><span className="text-left">camera (3rd / 1st)</span>
          </div>
          <div className="mt-5 max-w-md text-xs leading-5 text-zinc-400">
            Procedural island, destructible buildings. Every browser tab is a
            player — buildings you demolish crumble for everyone, live.
          </div>
          {bootError && (
            <div className="mt-4 max-w-md rounded bg-red-950/70 px-3 py-2 font-mono text-xs text-red-300">
              {bootError}
            </div>
          )}
        </button>
      )}

      {/* Bottom-left: minimap */}
      {game && stats && <Minimap game={game} stats={stats} />}

      {/* Bottom-right: reset */}
      <div className="absolute bottom-4 right-4">
        <button
          type="button"
          onClick={resetIsland}
          className="rounded-md bg-black/45 px-3 py-1.5 font-mono text-xs text-zinc-300 backdrop-blur-sm transition-colors hover:bg-black/65 hover:text-white"
        >
          rebuild island
        </button>
      </div>

      {game && userId && <SyncBridge game={game} userId={userId} />}
    </div>
  );
}

function HudRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-2 tabular-nums">
      <span className="w-14 text-zinc-400">{label}</span>
      <span>{value}</span>
    </div>
  );
}

const WORLD_HALF = 256; // mirror of game/config.ts — minimap projection only

/**
 * Bottom-left minimap: the island image is rendered once by the game
 * (heightfield + hillshade + building footprints); this component just
 * composites the live markers over it at the HUD's 4 Hz cadence.
 */
function Minimap({ game, stats }: { game: Game; stats: GameStats }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const baseRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    baseRef.current = game.buildMinimapImage();
  }, [game]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const base = baseRef.current;
    if (!canvas || !base) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const size = canvas.width;
    const toPx = (w: number) => ((w + WORLD_HALF) / (2 * WORLD_HALF)) * size;

    ctx.clearRect(0, 0, size, size);
    ctx.drawImage(base, 0, 0, size, size);

    // Remote players.
    for (const dot of stats.remotes) {
      ctx.fillStyle = dot.color;
      ctx.beginPath();
      ctx.arc(toPx(dot.x), toPx(dot.z), 3, 0, Math.PI * 2);
      ctx.fill();
    }

    // Self: white arrow pointing along the view heading. World yaw 0
    // looks down -z (map up), so rotate by -heading.
    ctx.save();
    ctx.translate(toPx(stats.pose.x), toPx(stats.pose.z));
    ctx.rotate(-stats.pose.heading);
    ctx.fillStyle = "#ffffff";
    ctx.strokeStyle = "rgba(0,0,0,0.7)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(0, -6);
    ctx.lineTo(4.5, 5);
    ctx.lineTo(0, 2.5);
    ctx.lineTo(-4.5, 5);
    ctx.closePath();
    ctx.stroke();
    ctx.fill();
    ctx.restore();
  }, [stats]);

  return (
    <div className="absolute bottom-4 left-4 overflow-hidden rounded-lg border border-white/15 bg-black/40 shadow-lg">
      <canvas ref={canvasRef} width={176} height={176} className="block h-44 w-44" />
    </div>
  );
}
