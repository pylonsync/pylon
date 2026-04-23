/**
 * Pylon Trade — live market dashboard.
 *
 * Layout:
 *   - Topbar: ticks/sec, active symbols, watchlist size, "Start ticker"
 *   - Main: top movers table (sorted by % change) + watchlist column
 *   - Right: selected symbol detail with client-computed sparkline
 *
 * One tab should click "Start ticker" — it generates trades for all
 * symbols at ~100 Hz. Every other tab passively subscribes to the
 * Ticker table and sees prices update live.
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
init({ baseUrl: BASE_URL, appName: "trade" });
configureClient({ baseUrl: BASE_URL, appName: "trade" });

type Ticker = {
  id: string;
  symbol: string;
  name: string;
  sector: string;
  price: number;
  openPrice: number;
  dayHigh: number;
  dayLow: number;
  volume: number;
  updatedAt: string;
};

type Trade = {
  id: string;
  symbol: string;
  price: number;
  qty: number;
  at: string;
};

type Watch = {
  id: string;
  userId: string;
  symbol: string;
  addedAt: string;
};

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

export function TradeApp() {
  const [userId, setUserId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [ticksPerSec, setTicksPerSec] = useState(0);

  const { data: tickers } = db.useQuery<Ticker>("Ticker");
  const { data: watches } = db.useQuery<Watch>("Watch", userId ? { where: { userId } } : undefined);

  // Sparkline data — recent Trade rows for the selected symbol.
  const { data: recentTrades } = db.useQuery<Trade>(
    "Trade",
    selected ? { where: { symbol: selected } } : undefined,
  );

  const tickCount = useRef(0);

  // One-time init: auth + seed the market.
  useEffect(() => {
    let cancelled = false;
    ensureGuest().then(async (id) => {
      if (cancelled) return;
      setUserId(id);
      try {
        await callFn("seedMarket", {});
      } catch (e) {
        console.error("seedMarket failed", e);
      }
    });
    return () => { cancelled = true; };
  }, []);

  // Ticker driver — writes trades in a tight loop while running is on.
  useEffect(() => {
    if (!running || !tickers || tickers.length === 0) return;
    let cancelled = false;

    const step = async () => {
      if (cancelled) return;
      // Emit 20 trades per frame to keep a steady high rate without
      // blocking the UI. Picks symbols with random walk price moves.
      const batch: Promise<unknown>[] = [];
      for (let i = 0; i < 20; i++) {
        const t = tickers[Math.floor(Math.random() * tickers.length)];
        const move = (Math.random() - 0.5) * t.price * 0.002; // ±0.2%
        const price = Math.max(0.01, +(t.price + move).toFixed(2));
        const qty = Math.floor(1 + Math.random() * 40) * 100;
        batch.push(
          callFn("recordTrade", { symbol: t.symbol, price, qty }).catch(() => {}),
        );
      }
      await Promise.all(batch);
      tickCount.current += 20;
      if (!cancelled) setTimeout(step, 120);
    };
    step();
    return () => { cancelled = true; };
  }, [running, tickers]);

  // HUD tick rate refresh.
  useEffect(() => {
    const t = setInterval(() => {
      setTicksPerSec(tickCount.current);
      tickCount.current = 0;
    }, 1000);
    return () => clearInterval(t);
  }, []);

  const watchSet = useMemo(
    () => new Set((watches ?? []).map((w) => w.symbol)),
    [watches],
  );

  const sorted = useMemo(() => {
    const list = tickers ?? [];
    const withChange = list.map((t) => ({
      ...t,
      pct: t.openPrice > 0 ? ((t.price - t.openPrice) / t.openPrice) * 100 : 0,
    }));
    return withChange.sort((a, b) => Math.abs(b.pct) - Math.abs(a.pct));
  }, [tickers]);

  const selectedTicker = selected
    ? (tickers ?? []).find((t) => t.symbol === selected)
    : null;

  async function toggleWatch(symbol: string) {
    if (!userId) return;
    await callFn("toggleWatch", { userId, symbol }).catch(() => {});
  }

  return (
    <div className="tr-app">
      <div className="tr-topbar">
        <div className="tr-brand">
          <svg viewBox="0 0 48 64" width="18" height="24" fill="currentColor">
            <path d="M24 2 L10 20 L24 32 Z" />
            <path d="M24 2 L38 20 L24 32 Z" />
            <path d="M24 32 L18 48 L24 62 L30 48 Z" />
            <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
            <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
          </svg>
          <span>Pylon · Trade</span>
        </div>
        <div className="tr-stats">
          <div className="tr-stat">
            <span className="tr-stat-label">TICKS/S</span>
            <span className="tr-stat-value">{ticksPerSec}</span>
          </div>
          <div className="tr-stat">
            <span className="tr-stat-label">SYMBOLS</span>
            <span className="tr-stat-value">{(tickers ?? []).length}</span>
          </div>
          <div className="tr-stat">
            <span className="tr-stat-label">WATCH</span>
            <span className="tr-stat-value">{(watches ?? []).length}</span>
          </div>
        </div>
        <div className="tr-actions">
          <button
            className={`tr-btn ${running ? "active" : "primary"}`}
            onClick={() => setRunning((r) => !r)}
          >
            {running ? "Stop ticker" : "Start ticker"}
          </button>
        </div>
      </div>

      <div className="tr-body">
        <div className="tr-col tr-col-watch">
          <div className="tr-col-head">Watchlist</div>
          {(watches ?? []).length === 0 && (
            <div className="tr-empty">Click a row → star to watch.</div>
          )}
          {(watches ?? []).map((w) => {
            const t = (tickers ?? []).find((tt) => tt.symbol === w.symbol);
            if (!t) return null;
            const pct = t.openPrice > 0 ? ((t.price - t.openPrice) / t.openPrice) * 100 : 0;
            return (
              <div
                key={w.id}
                className={`tr-watch-row ${selected === t.symbol ? "active" : ""}`}
                onClick={() => setSelected(t.symbol)}
              >
                <span className="tr-watch-sym">{t.symbol}</span>
                <span className="tr-watch-price">{t.price.toFixed(2)}</span>
                <span className={`tr-watch-pct ${pct >= 0 ? "up" : "down"}`}>
                  {pct >= 0 ? "+" : ""}{pct.toFixed(2)}%
                </span>
              </div>
            );
          })}
        </div>

        <div className="tr-col tr-col-main">
          <div className="tr-col-head">Top movers</div>
          <div className="tr-table">
            <div className="tr-row tr-row-head">
              <span>Symbol</span>
              <span>Name</span>
              <span>Sector</span>
              <span className="tr-num">Price</span>
              <span className="tr-num">Day ±</span>
              <span className="tr-num">Volume</span>
              <span />
            </div>
            {sorted.map((t) => {
              const pct = t.pct;
              return (
                <div
                  key={t.id}
                  className={`tr-row ${selected === t.symbol ? "active" : ""}`}
                  onClick={() => setSelected(t.symbol)}
                >
                  <span className="tr-sym">{t.symbol}</span>
                  <span className="tr-name">{t.name}</span>
                  <span className="tr-sector">{t.sector}</span>
                  <span className="tr-num">${t.price.toFixed(2)}</span>
                  <span className={`tr-num ${pct >= 0 ? "up" : "down"}`}>
                    {pct >= 0 ? "+" : ""}{pct.toFixed(2)}%
                  </span>
                  <span className="tr-num tr-vol">{t.volume.toLocaleString()}</span>
                  <button
                    className={`tr-star ${watchSet.has(t.symbol) ? "on" : ""}`}
                    onClick={(e) => { e.stopPropagation(); toggleWatch(t.symbol); }}
                    title={watchSet.has(t.symbol) ? "Unwatch" : "Watch"}
                  >{watchSet.has(t.symbol) ? "★" : "☆"}</button>
                </div>
              );
            })}
          </div>
        </div>

        <div className="tr-col tr-col-detail">
          <div className="tr-col-head">
            {selectedTicker ? selectedTicker.symbol : "—"}
          </div>
          {selectedTicker ? (
            <div className="tr-detail">
              <div className="tr-detail-name">{selectedTicker.name}</div>
              <div className="tr-detail-price">
                ${selectedTicker.price.toFixed(2)}
              </div>
              <div className="tr-detail-row">
                <span className="tr-stat-label">OPEN</span>
                <span>${selectedTicker.openPrice.toFixed(2)}</span>
              </div>
              <div className="tr-detail-row">
                <span className="tr-stat-label">HIGH</span>
                <span>${selectedTicker.dayHigh.toFixed(2)}</span>
              </div>
              <div className="tr-detail-row">
                <span className="tr-stat-label">LOW</span>
                <span>${selectedTicker.dayLow.toFixed(2)}</span>
              </div>
              <div className="tr-detail-row">
                <span className="tr-stat-label">VOL</span>
                <span>{selectedTicker.volume.toLocaleString()}</span>
              </div>
              <Sparkline trades={recentTrades ?? []} />
            </div>
          ) : (
            <div className="tr-empty">Pick a symbol.</div>
          )}
        </div>
      </div>
    </div>
  );
}

function Sparkline({ trades }: { trades: Trade[] }) {
  if (trades.length < 2) {
    return <div className="tr-empty tr-sparkline-empty">no trades yet</div>;
  }
  const sorted = [...trades].sort((a, b) => +new Date(a.at) - +new Date(b.at)).slice(-60);
  const prices = sorted.map((t) => t.price);
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const span = Math.max(0.01, max - min);
  const W = 240, H = 80;
  const points = prices.map((p, i) => {
    const x = (i / (prices.length - 1)) * W;
    const y = H - ((p - min) / span) * H;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(" ");
  return (
    <svg className="tr-sparkline" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
      <polyline
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        points={points}
      />
    </svg>
  );
}
