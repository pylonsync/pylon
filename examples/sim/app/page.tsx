import React, { useCallback, useEffect, useRef, useState } from "react";
import { callFn, db, init, storageKey } from "@pylonsync/react";
import type { City, CityStats, Tool } from "../game/city";

/**
 * Pylon Sim — co-op city builder on Pylon SSR.
 *
 * The page SSRs as a light shell, then a client-only effect
 * dynamic-imports the three.js city engine (it never loads during
 * SSR). Pylon live queries (<SyncBridge/>) feed the shared Tile + City
 * rows into the engine; the tool palette drives placement, which the
 * engine batches back to the server.
 */

interface TileRow {
  id: string;
  gx: number;
  gz: number;
  kind: string;
  level: number;
}
interface CityRow {
  id: string;
  key: string;
  funds: number;
  population: number;
  jobs: number;
  happiness: number;
  tick: number;
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
    try {
      const res = await fetch("/api/auth/session", {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.ok) {
        const body = (await res.json()) as { session?: { user_id?: string | null } };
        if (body.session?.user_id) return userId;
      }
    } catch {
      /* fall through to a fresh session */
    }
    window.localStorage.removeItem(storageKey("token"));
    window.localStorage.removeItem(storageKey("userId"));
  }
  return newGuestSession();
}

/** Pushes the shared Tile + City live queries into the engine. */
function SyncBridge({ game }: { game: City }) {
  const { data: tiles } = db.useQuery<TileRow>("Tile");
  const { data: city } = db.useQuery<CityRow>("City");

  useEffect(() => {
    if (tiles) game.setTiles(tiles);
  }, [game, tiles]);

  useEffect(() => {
    const main = city?.find((c) => c.key === "main");
    if (main) game.setCity(main);
  }, [game, city]);

  return null;
}

/** Inline lucide icons (MIT) — stroked, currentColor. */
const ICON_PATHS: Record<string, React.ReactNode> = {
  // road: signpost-free road with a dashed centreline
  road: (
    <>
      <path d="M4 19 8 5h8l4 14" />
      <path d="M12 6v2" />
      <path d="M12 11v2" />
      <path d="M12 16v2" />
    </>
  ),
  // home
  res: (
    <>
      <path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <path d="M9 22V12h6v10" />
    </>
  ),
  // building-2
  com: (
    <>
      <path d="M6 22V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v18Z" />
      <path d="M6 12H4a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2" />
      <path d="M18 9h2a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-2" />
      <path d="M10 6h4M10 10h4M10 14h4M10 18h4" />
    </>
  ),
  // factory
  ind: (
    <>
      <path d="M2 20a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8l-7 5V8l-7 5V4a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2Z" />
      <path d="M17 18h1M12 18h1M7 18h1" />
    </>
  ),
  // trash-2
  bulldoze: (
    <>
      <path d="M3 6h18" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
      <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
      <path d="M10 11v6M14 11v6" />
    </>
  ),
};

function ToolIcon({ id, color }: { id: string; color: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke={color}
      strokeWidth={1.9}
      strokeLinecap="round"
      strokeLinejoin="round"
      className="h-6 w-6"
      aria-hidden
    >
      {ICON_PATHS[id]}
    </svg>
  );
}

const TOOLS: Array<{ id: Tool; label: string; key: string; color: string }> = [
  { id: "road", label: "Road", key: "1", color: "#c8d0d8" },
  { id: "res", label: "Residential", key: "2", color: "#5ad05a" },
  { id: "com", label: "Commercial", key: "3", color: "#54b8e0" },
  { id: "ind", label: "Industrial", key: "4", color: "#e0b73c" },
  { id: "bulldoze", label: "Bulldoze", key: "5", color: "#ff6b5a" },
];

export default function SimPage() {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const gameRef = useRef<City | null>(null);
  const [game, setGame] = useState<City | null>(null);
  const [tool, setTool] = useState<Tool>("road");
  const [stats, setStats] = useState<CityStats | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    let disposed = false;
    init({ appName: "sim" });

    (async () => {
      const { City } = await import("../game/city");
      if (disposed) return;
      const g = new City(mount);
      gameRef.current = g;
      (window as unknown as Record<string, unknown>).__sim = { game: g };
      g.onStats(setStats);
      g.start();
      setGame(g);

      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          await ensureGuestSession();
          if (disposed) return;
          await callFn("ensureCity", {});
          if (!disposed) setBootError(null);
          break;
        } catch (err) {
          console.error(`[sim] boot failed (attempt ${attempt + 1})`, err);
          if (disposed) return;
          if (attempt === 4) setBootError(String(err));
          else await new Promise((r) => setTimeout(r, 700 * (attempt + 1)));
        }
      }
    })().catch((err) => {
      console.error("[sim] engine boot failed", err);
      if (!disposed) setBootError(String(err));
    });

    return () => {
      disposed = true;
      gameRef.current?.dispose();
      gameRef.current = null;
    };
  }, []);

  const pick = useCallback((t: Tool) => {
    setTool(t);
    gameRef.current?.setTool(t);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = TOOLS.find((x) => x.key === e.key);
      if (t) pick(t.id);
      if (e.key.toLowerCase() === "b") pick("bulldoze");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pick]);

  const resetCity = useCallback(() => {
    callFn("resetCity", {}).catch(() => {});
  }, []);

  return (
    <div className="fixed inset-0">
      <div ref={mountRef} className="absolute inset-0" />

      {/* Top-left: economy + perf */}
      <div className="absolute left-4 top-4 select-none rounded-lg bg-black/55 px-4 py-3 font-mono text-xs leading-5 text-zinc-200 backdrop-blur-sm">
        <div className="mb-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-400">
          Pylon · Sim
        </div>
        {stats ? (
          <>
            <Stat label="funds" value={`$${Math.round(stats.funds).toLocaleString()}`} />
            <Stat label="pop" value={`${Math.round(stats.population).toLocaleString()}`} />
            <Stat label="jobs" value={`${Math.round(stats.jobs).toLocaleString()}`} />
            <Stat label="mood" value={`${Math.round(stats.happiness)}%`} />
            <div className="my-1 h-px bg-white/10" />
            <Stat label="fps" value={`${stats.fps}`} />
            <Stat label="draws" value={`${stats.draws}`} />
            <Stat label="tiles" value={`${stats.tiles}`} />
            <Stat label="sync" value={`${stats.mutPerSec}/s`} />
          </>
        ) : (
          <div className="text-zinc-400">starting city…</div>
        )}
      </div>

      {/* Top-right: title / co-op hint */}
      <div className="absolute right-4 top-4 max-w-xs select-none rounded-lg bg-black/45 px-4 py-3 text-right text-xs text-zinc-300 backdrop-blur-sm">
        <div className="text-sm font-semibold text-white">Co-op City</div>
        <div className="mt-1 leading-5 text-zinc-400">
          Every tab is a co-mayor of one shared city. Roads, zones and the
          skyline sync live. Zones only grow next to a road.
        </div>
      </div>

      {/* Bottom-center: tool palette */}
      <div className="absolute bottom-5 left-1/2 -translate-x-1/2 select-none">
        <div className="flex items-end gap-2 rounded-2xl bg-black/55 p-2 backdrop-blur-md">
          {TOOLS.map((t) => {
            const active = tool === t.id;
            return (
              <button
                key={t.id}
                type="button"
                onClick={() => pick(t.id)}
                className={`flex w-20 flex-col items-center gap-1 rounded-xl px-2 py-2 transition-all ${
                  active ? "bg-white/15 ring-1 ring-white/40" : "hover:bg-white/10"
                }`}
                style={active ? { boxShadow: `0 0 0 1px ${t.color}55` } : undefined}
              >
                <ToolIcon id={t.id} color={t.color} />
                <span className="font-mono text-[11px] text-zinc-200">{t.label}</span>
                <span className="font-mono text-[9px] text-zinc-500">{t.key}</span>
              </button>
            );
          })}
        </div>
        <div className="mt-2 text-center font-mono text-[10px] text-zinc-500">
          left-click / drag to build · right-drag to pan · scroll to zoom · Q/E rotate
        </div>
      </div>

      {/* Bottom-right: reset */}
      <div className="absolute bottom-5 right-4">
        <button
          type="button"
          onClick={resetCity}
          className="rounded-md bg-black/45 px-3 py-1.5 font-mono text-xs text-zinc-300 backdrop-blur-sm transition-colors hover:bg-black/65 hover:text-white"
        >
          new city
        </button>
      </div>

      {bootError && (
        <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded bg-red-950/80 px-4 py-3 font-mono text-xs text-red-200">
          {bootError}
        </div>
      )}

      {game && <SyncBridge game={game} />}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4 tabular-nums">
      <span className="text-zinc-400">{label}</span>
      <span>{value}</span>
    </div>
  );
}
