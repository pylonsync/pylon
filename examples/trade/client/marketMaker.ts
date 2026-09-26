import { useEffect, useRef } from "react";
import { callFn } from "@pylonsync/react";
import type { Ticker } from "./types";

/** Trades per second the market maker aims for. */
export const TARGET_RATE = 2_000;
/**
 * Trades per `recordTrades` call. A call changes about 27 synced rows (one
 * Ticker per symbol, the tape slots, the Market row) however many trades
 * it carries. Every change reaches every subscribed tab, so fewer, larger
 * batches keep each replica caught up.
 */
export const BATCH_SIZE = 500;
/** Batches allowed in flight at once. */
export const MAX_IN_FLIGHT = 2;
/** Each sector's drift is redrawn on this interval. */
const REGIME_MS = 6_000;
/** Standard deviation of each symbol's one-second return. */
const VOL_PER_SEC = 0.0016;
/** Largest sector drift, as a return per second. */
const MAX_DRIFT_PER_SEC = 0.0006;

export type WriteStats = {
  /** Round-trip times of recent `recordTrades` calls, in ms. */
  rtts: number[];
  /** Failed calls since the ticker started. */
  errors: number;
};

/**
 * Client-side market maker. While `running`, it sends batches of random-walk
 * fills to `recordTrades` at TARGET_RATE trades per second. Each symbol's
 * price walks from its last known price with the same per-second volatility
 * whatever its trade count. Each sector has a drift term that changes every
 * REGIME_MS, so sectors move together.
 */
export function useMarketMaker(running: boolean, tickers: Ticker[]) {
  const stats = useRef<WriteStats>({ rtts: [], errors: 0 });
  const tickersRef = useRef(tickers);
  tickersRef.current = tickers;

  useEffect(() => {
    if (!running) return;
    const universe = tickersRef.current;
    if (universe.length === 0) return;

    const prices = new Map(universe.map((t) => [t.symbol, t.price]));
    // Busier names trade more often: the weight falls with list position.
    const weights = universe.map((_, i) => 1 / (1 + i * 0.12));
    const totalWeight = weights.reduce((a, b) => a + b, 0);
    // Expected fills per second for each symbol.
    const perSec = new Map(
      universe.map((t, i) => [t.symbol, (TARGET_RATE * weights[i]) / totalWeight]),
    );
    const sectors = [...new Set(universe.map((t) => t.sector))];
    const drift = new Map<string, number>();
    const redraw = () => {
      for (const s of sectors) {
        drift.set(s, (Math.random() * 2 - 1) * MAX_DRIFT_PER_SEC);
      }
    };
    redraw();
    let regimeAt = performance.now();

    const pick = (): Ticker => {
      let r = Math.random() * totalWeight;
      for (let i = 0; i < universe.length; i++) {
        r -= weights[i];
        if (r <= 0) return universe[i];
      }
      return universe[universe.length - 1];
    };

    const makeBatch = () => {
      const trades: Array<{ symbol: string; price: number; qty: number }> = [];
      for (let i = 0; i < BATCH_SIZE; i++) {
        const t = pick();
        const last = prices.get(t.symbol) ?? t.price;
        const n = perSec.get(t.symbol) ?? 1;
        // Uniform noise on [-a, a] has standard deviation a / sqrt(3).
        const noise = (Math.random() * 2 - 1) * Math.sqrt(3) * (VOL_PER_SEC / Math.sqrt(n));
        const step = (drift.get(t.sector) ?? 0) / n + noise;
        const price = Math.max(0.01, +(last * (1 + step)).toFixed(2));
        prices.set(t.symbol, price);
        // Lower-priced names trade in larger size.
        const lots = Math.ceil((1 + Math.random() * 30) * Math.sqrt(100 / last));
        trades.push({ symbol: t.symbol, price, qty: lots * 100 });
      }
      return trades;
    };

    let cancelled = false;
    let inFlight = 0;
    let owed = 0;
    let lastTick = performance.now();

    const send = async () => {
      inFlight++;
      const t0 = performance.now();
      try {
        await callFn("recordTrades", { trades: makeBatch() });
        const rtts = stats.current.rtts;
        rtts.push(performance.now() - t0);
        if (rtts.length > 200) rtts.shift();
      } catch {
        stats.current.errors++;
      } finally {
        inFlight--;
      }
    };

    const timer = window.setInterval(() => {
      if (cancelled) return;
      const now = performance.now();
      if (now - regimeAt > REGIME_MS) {
        redraw();
        regimeAt = now;
      }
      // Cap the backlog at one second so a stalled server does not
      // trigger a burst when it recovers.
      owed = Math.min(owed + ((now - lastTick) / 1000) * TARGET_RATE, TARGET_RATE);
      lastTick = now;
      while (owed >= BATCH_SIZE && inFlight < MAX_IN_FLIGHT) {
        owed -= BATCH_SIZE;
        void send();
      }
    }, 10);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [running]);

  return stats;
}
