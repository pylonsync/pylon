import React, { useCallback, useEffect, useRef, useState } from "react";
import { callFn, db, init, storageKey } from "@pylonsync/react";
import type { City, CityStats, Tool } from "../game/city";

/**
 * Pylon Sim — co-op city builder on Pylon SSR.
 *
 * Opens to a LOBBY: start a new city or join an existing one (every city is
 * its own co-op map). Picking a city boots the three.js engine scoped to that
 * city; Pylon live queries feed only that city's Tile + City rows into it.
 * The page SSRs as a light shell — the engine is dynamic-imported client-side
 * and never loads during SSR.
 */

interface TileRow {
  id: string;
  cityId: string;
  gx: number;
  gz: number;
  kind: string;
  level: number;
}
interface CityRow {
  id: string;
  key: string;
  name: string;
  funds: number;
  population: number;
  jobs: number;
  happiness: number;
  tick: number;
  updatedAt: string;
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

// Initialise the pylon client once, on the client, before any db hook
// subscribes. Done in render (guarded) rather than an effect so the lobby's
// City query namespaces correctly on its very first read.
let _inited = false;
function useClientInit() {
  if (typeof window !== "undefined" && !_inited) {
    init({ appName: "sim" });
    _inited = true;
  }
}

// ---------------------------------------------------------------------------
// Lobby — start a new city or join an existing one
// ---------------------------------------------------------------------------

function Lobby({
  authReady,
  authError,
  onEnter,
}: {
  authReady: boolean;
  authError: string | null;
  onEnter: (cityId: string) => void;
}) {
  const { data: cities, loading } = db.useQuery<CityRow>("City", {
    orderBy: { updatedAt: "desc" },
  });
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Make sure the always-on demo city exists so "Load a city" is never empty
  // on a fresh server (idempotent server-side; safe to fire on every visit).
  useEffect(() => {
    if (!authReady) return;
    callFn("seedDemoCity", {}).catch(() => {});
  }, [authReady]);

  const create = useCallback(async () => {
    if (busy || !authReady) return;
    setBusy(true);
    setErr(null);
    try {
      const res = await callFn<{ cityId: string }>("createCity", {
        name: name.trim() || "New City",
      });
      if (!res?.cityId) throw new Error("city creation returned no id");
      onEnter(res.cityId);
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  }, [busy, authReady, name, onEnter]);

  const load = useCallback(
    async (cityId: string) => {
      if (busy) return;
      setBusy(true);
      // Revive the city's sim chain (no-op if already running), then enter.
      try {
        await callFn("ensureCity", { cityId });
      } catch {
        /* the city still loads; the tick revives on next interaction */
      }
      onEnter(cityId);
    },
    [busy, onEnter],
  );

  return (
    <div className="fixed inset-0 flex items-center justify-center bg-gradient-to-b from-[#0b1018] to-[#070b11] p-6 text-zinc-100">
      <div className="w-full max-w-md rounded-2xl border border-white/10 bg-black/40 p-7 shadow-2xl backdrop-blur-md">
        <div className="mb-1 font-mono text-[10px] font-semibold uppercase tracking-[0.3em] text-zinc-500">
          Pylon · Sim
        </div>
        <h1 className="text-2xl font-semibold tracking-tight text-white">Co-op City Builder</h1>
        <p className="mt-1 text-sm leading-6 text-zinc-400">
          Start a new city or jump into one already being built. Every city is a
          shared map — anyone who joins is a co-mayor, live.
        </p>

        {/* New city */}
        <div className="mt-6">
          <label className="mb-1.5 block font-mono text-[11px] uppercase tracking-wider text-zinc-500">
            New city
          </label>
          <div className="flex gap-2">
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") create();
              }}
              maxLength={40}
              placeholder="Name your city…"
              className="min-w-0 flex-1 rounded-lg border border-white/10 bg-black/40 px-3 py-2.5 text-sm text-white placeholder:text-zinc-600 outline-none focus:border-emerald-400/60"
            />
            <button
              type="button"
              onClick={create}
              disabled={busy || !authReady}
              className="shrink-0 rounded-lg bg-emerald-500 px-4 py-2.5 text-sm font-semibold text-emerald-950 transition-colors hover:bg-emerald-400 disabled:cursor-not-allowed disabled:opacity-40"
            >
              {busy ? "…" : "Create"}
            </button>
          </div>
        </div>

        {/* Existing cities */}
        <div className="mt-6">
          <label className="mb-1.5 block font-mono text-[11px] uppercase tracking-wider text-zinc-500">
            Load a city
          </label>
          <div className="max-h-64 space-y-1.5 overflow-y-auto pr-1">
            {loading ? (
              <div className="px-1 py-3 text-sm text-zinc-500">loading cities…</div>
            ) : cities.length === 0 ? (
              <div className="px-1 py-3 text-sm text-zinc-500">
                No cities yet. Name one above to begin.
              </div>
            ) : (
              cities.map((c) => (
                <button
                  key={c.key}
                  type="button"
                  onClick={() => load(c.key)}
                  disabled={busy}
                  className="flex w-full items-center justify-between gap-3 rounded-lg border border-white/5 bg-white/[0.03] px-3 py-2.5 text-left transition-colors hover:border-white/15 hover:bg-white/[0.07] disabled:opacity-50"
                >
                  <span className="truncate text-sm font-medium text-zinc-100">{c.name}</span>
                  <span className="shrink-0 font-mono text-[11px] tabular-nums text-zinc-500">
                    {Math.round(c.population).toLocaleString()} pop
                  </span>
                </button>
              ))
            )}
          </div>
        </div>

        {(err || authError) && (
          <div className="mt-4 rounded-lg bg-red-950/70 px-3 py-2 font-mono text-xs text-red-200">
            {err ?? authError}
          </div>
        )}
        {!authReady && !authError && (
          <div className="mt-4 font-mono text-[11px] text-zinc-600">connecting…</div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Tool palette icons (inline lucide, MIT)
// ---------------------------------------------------------------------------

const ICON_PATHS: Record<string, React.ReactNode> = {
  road: (
    <>
      <path d="M4 19 8 5h8l4 14" />
      <path d="M12 6v2" />
      <path d="M12 11v2" />
      <path d="M12 16v2" />
    </>
  ),
  res: (
    <>
      <path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      <path d="M9 22V12h6v10" />
    </>
  ),
  com: (
    <>
      <path d="M6 22V4a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v18Z" />
      <path d="M6 12H4a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h2" />
      <path d="M18 9h2a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-2" />
      <path d="M10 6h4M10 10h4M10 14h4M10 18h4" />
    </>
  ),
  ind: (
    <>
      <path d="M2 20a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8l-7 5V8l-7 5V4a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2Z" />
      <path d="M17 18h1M12 18h1M7 18h1" />
    </>
  ),
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

// ---------------------------------------------------------------------------
// GameView — the three.js city, scoped to one cityId
// ---------------------------------------------------------------------------

function GameView({ cityId, onLeave }: { cityId: string; onLeave: () => void }) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const gameRef = useRef<City | null>(null);
  const [game, setGame] = useState<City | null>(null);
  const [tool, setTool] = useState<Tool>("road");
  const [stats, setStats] = useState<CityStats | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);

  // Live data for THIS city only.
  const { data: tiles } = db.useQuery<TileRow>("Tile", { where: { cityId } });
  const { data: cityRows } = db.useQuery<CityRow>("City", { where: { key: cityId } });
  const cityName = cityRows?.[0]?.name ?? "City";

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    let disposed = false;

    (async () => {
      const { City } = await import("../game/city");
      if (disposed) return;
      const g = new City(mount, cityId);
      gameRef.current = g;
      (window as unknown as Record<string, unknown>).__sim = { game: g };
      g.onStats(setStats);
      g.start();
      setGame(g);

      // Make sure this city's simulation chain is alive (revive after a
      // server restart). Retry a few times behind the guest session.
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          await callFn("ensureCity", { cityId });
          if (!disposed) setBootError(null);
          break;
        } catch (err) {
          console.error(`[sim] ensureCity failed (attempt ${attempt + 1})`, err);
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
      (window as unknown as Record<string, unknown>).__sim = undefined;
    };
  }, [cityId]);

  // Feed live data into the engine once it's up.
  useEffect(() => {
    if (game && tiles) game.setTiles(tiles);
  }, [game, tiles]);
  useEffect(() => {
    const c = cityRows?.[0];
    if (game && c) game.setCity(c);
  }, [game, cityRows]);

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
    callFn("resetCity", { cityId }).catch(() => {});
  }, [cityId]);

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

      {/* Top-right: city name / co-op hint */}
      <div className="absolute right-4 top-4 max-w-xs select-none rounded-lg bg-black/45 px-4 py-3 text-right text-xs text-zinc-300 backdrop-blur-sm">
        <div className="truncate text-sm font-semibold text-white">{cityName}</div>
        <div className="mt-1 leading-5 text-zinc-400">
          Every tab is a co-mayor of this city. Roads, zones and the skyline
          sync live. Zones only grow next to a road.
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

      {/* Bottom-right: re-roll layout + back to lobby */}
      <div className="absolute bottom-5 right-4 flex gap-2">
        <button
          type="button"
          onClick={resetCity}
          className="rounded-md bg-black/45 px-3 py-1.5 font-mono text-xs text-zinc-300 backdrop-blur-sm transition-colors hover:bg-black/65 hover:text-white"
        >
          reset
        </button>
        <button
          type="button"
          onClick={onLeave}
          className="rounded-md bg-black/45 px-3 py-1.5 font-mono text-xs text-zinc-300 backdrop-blur-sm transition-colors hover:bg-black/65 hover:text-white"
        >
          ← menu
        </button>
      </div>

      {bootError && (
        <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded bg-red-950/80 px-4 py-3 font-mono text-xs text-red-200">
          {bootError}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// SimPage — lobby ↔ game, with ?city= resume
// ---------------------------------------------------------------------------

export default function SimPage() {
  useClientInit();
  const [authReady, setAuthReady] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [activeCityId, setActiveCityId] = useState<string | null>(null);
  const [hydrated, setHydrated] = useState(false);

  // Establish a guest session once.
  useEffect(() => {
    let alive = true;
    (async () => {
      for (let attempt = 0; attempt < 5; attempt++) {
        try {
          await ensureGuestSession();
          if (alive) setAuthReady(true);
          return;
        } catch (err) {
          if (attempt === 4) {
            if (alive) setAuthError(String(err));
          } else {
            await new Promise((r) => setTimeout(r, 700 * (attempt + 1)));
          }
        }
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  // Resume into ?city= if present (so a refresh mid-game doesn't dump you back
  // to the lobby), but a fresh open with no param shows the lobby. Done in an
  // effect to keep SSR and the first client render identical (no mismatch).
  useEffect(() => {
    const id = new URLSearchParams(window.location.search).get("city");
    if (id) setActiveCityId(id);
    setHydrated(true);
  }, []);

  const enter = useCallback((cityId: string) => {
    setActiveCityId(cityId);
    const u = new URL(window.location.href);
    u.searchParams.set("city", cityId);
    window.history.replaceState({}, "", u);
  }, []);

  const leave = useCallback(() => {
    setActiveCityId(null);
    const u = new URL(window.location.href);
    u.searchParams.delete("city");
    window.history.replaceState({}, "", u);
  }, []);

  if (!hydrated) return <div className="fixed inset-0 bg-[#0a0e14]" />;
  if (!activeCityId) {
    return <Lobby authReady={authReady} authError={authError} onEnter={enter} />;
  }
  return <GameView key={activeCityId} cityId={activeCityId} onLeave={leave} />;
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-4 tabular-nums">
      <span className="text-zinc-400">{label}</span>
      <span>{value}</span>
    </div>
  );
}
