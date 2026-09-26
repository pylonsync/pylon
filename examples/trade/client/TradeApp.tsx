"use client";

/**
 * Pylon Trade: live market dashboard.
 *
 * Every panel reads live queries over the local replica:
 *   - Ticker rows: market table, heatmap, breadth, gainers and losers, and
 *     each symbol's open one-second candle
 *   - Bar rows (closed one-second candles): sparklines, the candle chart,
 *     fills per second
 *   - Fill rows: the trade tape
 *   - Market row: exchange-wide counters and the sync lag sample
 *
 * One tab clicks "Start ticker" and runs the market maker (marketMaker.ts).
 * Every other tab only subscribes. Heavy panels re-render from a snapshot
 * taken at most every RENDER_MS, so a burst of changes costs one paint.
 */
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { db, callFn, storageKey } from "@pylonsync/react";
import { Star } from "lucide-react";
import { cn } from "@pylonsync/example-ui/utils";
import { CandleChart, SectorTreemap, Sparkline, ThroughputChart } from "./charts";
import { MAX_IN_FLIGHT, BATCH_SIZE, TARGET_RATE, useMarketMaker } from "./marketMaker";
import {
  fmtClock,
  fmtCompact,
  fmtInt,
  fmtPct,
  fmtPrice,
  fmtSigned,
  percentile,
} from "./format";
import type { Bar, Candle, Fill, Market, Quote, Ticker, Watch } from "./types";

/** Minimum time between dashboard repaints, in ms. */
const RENDER_MS = 80;
/** Seconds of history on the candle and throughput charts. */
const CHART_SECONDS = 90;
/** Seconds of history in each table sparkline. */
const SPARK_SECONDS = 60;
/** Rows on the trade tape. */
const TAPE_ROWS = 22;
const DEFAULT_SYMBOL = "PYLO";

function getStoredUserId(): string | null {
  if (typeof window === "undefined") return null;
  return localStorage.getItem(storageKey("user"));
}

/** `value`, updated at most once per `ms`. The last change always lands. */
function useThrottled<T>(value: T, ms: number): T {
  const [out, setOut] = useState(value);
  const latest = useRef(value);
  const lastAt = useRef(0);
  const timer = useRef<number | null>(null);
  latest.current = value;

  useEffect(() => {
    if (timer.current !== null) return;
    const wait = Math.max(0, ms - (performance.now() - lastAt.current));
    timer.current = window.setTimeout(() => {
      timer.current = null;
      lastAt.current = performance.now();
      setOut(latest.current);
    }, wait);
  }, [value, ms]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );
  return out;
}

/** Current time, ticking once per second. */
function useNow() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  return now;
}

// ---------------------------------------------------------------------------

export function TradeApp() {
  const userId = getStoredUserId();
  const [running, setRunning] = useState(false);
  const [selected, setSelected] = useState<string>(DEFAULT_SYMBOL);
  const [seedError, setSeedError] = useState<string | null>(null);

  const { data: tickers, loading } = db.useQuery<Ticker>("Ticker");
  const { data: bars } = db.useQuery<Bar>("Bar");
  const { data: fills } = db.useQuery<Fill>("Fill", {
    orderBy: { seq: "desc" },
    limit: TAPE_ROWS,
  });
  const { data: markets } = db.useQuery<Market>("Market");
  const { data: watches } = db.useQuery<Watch>(
    "Watch",
    userId ? { where: { userId } } : undefined,
  );
  const market = markets[0] ?? null;

  const writeStats = useMarketMaker(running, tickers);

  useEffect(() => {
    let cancelled = false;
    callFn("seedMarket", {}).catch((e: unknown) => {
      if (!cancelled) setSeedError(e instanceof Error ? e.message : "Failed to seed market");
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Time from the write that last touched the Market row to this replica
  // seeing it. Both clocks are the same machine in local dev.
  const lagSamples = useRef<number[]>([]);
  useEffect(() => {
    if (!market) return;
    const lag = Date.now() - Date.parse(market.updatedAt);
    if (lag >= 0 && lag < 60_000) {
      lagSamples.current.push(lag);
      if (lagSamples.current.length > 200) lagSamples.current.shift();
    }
  }, [market?.updatedAt]);

  const live = useMemo(() => ({ tickers, bars, fills, market }), [tickers, bars, fills, market]);
  const snap = useThrottled(live, RENDER_MS);
  const now = useNow();

  const watchSet = useMemo(() => new Set(watches.map((w) => w.symbol)), [watches]);

  const quotes = useMemo<Quote[]>(() => {
    const list = snap.tickers.map((t) => {
      const change = t.price - t.openPrice;
      return { ...t, change, pct: t.openPrice > 0 ? (change / t.openPrice) * 100 : 0 };
    });
    return list.sort((a, b) => a.sector.localeCompare(b.sector) || a.symbol.localeCompare(b.symbol));
  }, [snap.tickers]);

  const series = useMemo(() => buildSeries(snap.bars, snap.tickers), [snap.bars, snap.tickers]);
  // A market that has not traded for three seconds reads as idle.
  const idle = series.end === 0 || now - series.end > 3_000;
  // Mean of the last three closed seconds. The newest second is still open.
  const closedSeconds = series.throughput.slice(-4, -1);
  const ticksPerSec = idle
    ? 0
    : Math.round(closedSeconds.reduce((a, b) => a + b, 0) / Math.max(1, closedSeconds.length));

  const selectedQuote = quotes.find((q) => q.symbol === selected) ?? null;
  const breadth = useMemo(() => computeBreadth(quotes, series.bySymbol, series.end), [quotes, series]);

  const rtts = writeStats.current.rtts;
  const stats = {
    ticksPerSec,
    trades: snap.market?.trades ?? 0,
    notional: snap.market?.notional ?? 0,
    advancing: breadth.advancing,
    declining: breadth.declining,
    writeP50: running ? percentile(rtts, 50) : null,
    writeP99: running ? percentile(rtts, 99) : null,
    lagP50: idle ? null : percentile(lagSamples.current, 50),
    symbols: quotes.length,
  };

  async function toggleWatch(symbol: string) {
    // The server derives the user from ctx.auth.userId.
    await callFn("toggleWatch", { symbol }).catch(() => {});
  }

  return (
    <div className="grid h-screen grid-rows-[48px_minmax(0,1fr)] bg-background text-foreground">
      <Header
        stats={stats}
        now={now}
        running={running}
        loading={loading}
        onToggle={() => setRunning((r) => !r)}
      />
      {seedError && (
        <div className="fixed inset-x-0 top-12 z-50 border-b border-[var(--color-bear)]/30 bg-[#2a1214] px-4 py-2 text-xs text-[var(--color-bear)]">
          Market seed failed: {seedError}
        </div>
      )}
      <div className="grid min-h-0 grid-cols-[400px_minmax(0,1fr)_360px] gap-px bg-[var(--color-line)]">
        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto] gap-px">
          <MarketTable
            quotes={quotes}
            spark={series.bySymbol}
            end={series.end}
            selected={selected}
            watchSet={watchSet}
            loading={loading}
            onSelect={setSelected}
            onToggleWatch={toggleWatch}
          />
          <BreadthPanel breadth={breadth} />
        </div>
        <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_276px] gap-px">
          <ChartPanel
            quote={selectedQuote}
            candles={series.bySymbol.get(selected) ?? EMPTY}
            end={series.end || now - (now % 1000)}
          />
          <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_264px] gap-px">
            <ThroughputPanel series={series.throughput} end={series.end} trades={stats.trades} idle={idle} />
            <MoversPanel quotes={quotes} onSelect={setSelected} />
          </div>
        </div>
        <div className="grid min-h-0 grid-rows-[304px_minmax(0,1fr)] gap-px">
          <Panel title="Sector map" meta="Size: dollar volume. Color: change from open.">
            <div className="min-h-0 flex-1 px-2 pb-2">
              <SectorTreemap quotes={quotes} selected={selected} onSelect={setSelected} />
            </div>
          </Panel>
          <TapePanel fills={snap.fills} lastSeq={snap.market?.seq ?? 0} />
        </div>
      </div>
    </div>
  );
}

const EMPTY: Candle[] = [];

// ---------------------------------------------------------------------------
// Derived series
// ---------------------------------------------------------------------------

type Series = {
  /** Epoch ms of the newest bar second, or 0 with no bars. */
  end: number;
  /** One-second candles per symbol within the chart window, oldest first. */
  bySymbol: Map<string, Candle[]>;
  /** Fills per second across all symbols, CHART_SECONDS entries ending at `end`. */
  throughput: number[];
};

/** Closed Bar rows plus each Ticker's open candle, bucketed by second. */
function buildSeries(bars: Bar[], tickers: Ticker[]): Series {
  let end = 0;
  const parsed: Array<Candle & { symbol: string }> = [];
  const closed = new Set<string>();
  for (const b of bars) {
    const t = Date.parse(b.t);
    if (!Number.isFinite(t)) continue;
    if (t > end) end = t;
    parsed.push({ ...b, t });
    closed.add(`${b.symbol}@${t}`);
  }
  for (const k of tickers) {
    if (!k.barT || !k.barTrades) continue;
    const t = Date.parse(k.barT);
    if (!Number.isFinite(t) || closed.has(`${k.symbol}@${t}`)) continue;
    if (t > end) end = t;
    parsed.push({
      symbol: k.symbol,
      t,
      open: k.barOpen ?? k.price,
      high: k.barHigh ?? k.price,
      low: k.barLow ?? k.price,
      close: k.price,
      volume: k.barVolume ?? 0,
      trades: k.barTrades,
    });
  }
  const start = end - (CHART_SECONDS - 1) * 1000;
  const bySymbol = new Map<string, Candle[]>();
  const throughput = new Array<number>(CHART_SECONDS).fill(0);
  for (const c of parsed) {
    if (c.t < start) continue;
    const list = bySymbol.get(c.symbol) ?? [];
    list.push(c);
    bySymbol.set(c.symbol, list);
    throughput[Math.round((c.t - start) / 1000)] += c.trades;
  }
  for (const list of bySymbol.values()) list.sort((a, b) => a.t - b.t);
  return { end, bySymbol, throughput };
}

type Breadth = {
  advancing: number;
  declining: number;
  unchanged: number;
  upVolume: number;
  downVolume: number;
  sectors: Array<{ name: string; up: number; down: number; pct: number }>;
};

function computeBreadth(quotes: Quote[], bySymbol: Map<string, Candle[]>, end: number): Breadth {
  let advancing = 0;
  let declining = 0;
  let unchanged = 0;
  const sectors = new Map<string, { up: number; down: number; w: number; wp: number }>();
  for (const q of quotes) {
    if (q.change > 0) advancing++;
    else if (q.change < 0) declining++;
    else unchanged++;
    const s = sectors.get(q.sector) ?? { up: 0, down: 0, w: 0, wp: 0 };
    if (q.change > 0) s.up++;
    if (q.change < 0) s.down++;
    const dollars = Math.max(1, q.volume * q.price);
    s.w += dollars;
    s.wp += dollars * q.pct;
    sectors.set(q.sector, s);
  }
  let upVolume = 0;
  let downVolume = 0;
  const from = end - (SPARK_SECONDS - 1) * 1000;
  for (const list of bySymbol.values()) {
    for (const c of list) {
      if (c.t < from) continue;
      if (c.close >= c.open) upVolume += c.volume;
      else downVolume += c.volume;
    }
  }
  return {
    advancing,
    declining,
    unchanged,
    upVolume,
    downVolume,
    sectors: [...sectors]
      .map(([name, s]) => ({ name, up: s.up, down: s.down, pct: s.wp / s.w }))
      .sort((a, b) => b.pct - a.pct),
  };
}

// ---------------------------------------------------------------------------
// Shell
// ---------------------------------------------------------------------------

function Panel({
  title,
  meta,
  children,
  className,
}: {
  title: ReactNode;
  meta?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("flex min-h-0 flex-col bg-[var(--color-panel)]", className)}>
      <header className="flex h-8 shrink-0 items-center justify-between gap-3 px-3">
        <h2 className="text-[12px] font-medium text-foreground/90">{title}</h2>
        {meta && <span className="truncate text-[11px] text-[var(--color-dim)]">{meta}</span>}
      </header>
      {children}
    </section>
  );
}

function Up({ value, children, className }: { value: number; children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "font-mono",
        value > 0 ? "text-[var(--color-bull)]" : value < 0 ? "text-[var(--color-bear)]" : "text-muted-foreground",
        className,
      )}
    >
      {children}
    </span>
  );
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

type HeaderStats = {
  ticksPerSec: number;
  trades: number;
  notional: number;
  advancing: number;
  declining: number;
  writeP50: number | null;
  writeP99: number | null;
  lagP50: number | null;
  symbols: number;
};

const Header = memo(function Header({
  stats,
  now,
  running,
  loading,
  onToggle,
}: {
  stats: HeaderStats;
  now: number;
  running: boolean;
  loading: boolean;
  onToggle: () => void;
}) {
  const ms = (v: number | null) => (v === null ? "—" : `${Math.round(v)} ms`);
  return (
    <header className="flex items-center gap-6 border-b border-[var(--color-line)] bg-[var(--color-panel)] px-4">
      <div className="flex items-center gap-2.5">
        <BrandMark />
        <span className="text-[13px] font-semibold tracking-tight">Pylon Trade</span>
        <span className="font-mono text-[12px] text-muted-foreground">{fmtClock(now)}</span>
      </div>
      <div className="h-6 w-px bg-[var(--color-line)]" />
      <div className="flex items-center gap-6">
        <Kpi label="Ticks/s, 3 s avg" value={fmtInt(stats.ticksPerSec)} strong />
        <Kpi label="Trades" value={fmtInt(stats.trades)} />
        <Kpi label="Notional" value={`$${fmtCompact(stats.notional)}`} />
        <Kpi
          label="Adv / Dec"
          value={
            <>
              <span className="text-[var(--color-bull)]">{stats.advancing}</span>
              <span className="text-[var(--color-dim)]"> / </span>
              <span className="text-[var(--color-bear)]">{stats.declining}</span>
            </>
          }
        />
        <Kpi label="Write p50" value={ms(stats.writeP50)} />
        <Kpi label="Write p99" value={ms(stats.writeP99)} />
        <Kpi label="Sync lag p50" value={ms(stats.lagP50)} />
        <Kpi label="Symbols" value={fmtInt(stats.symbols)} />
      </div>
      <div className="ml-auto flex items-center gap-3">
        <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <span
            className={cn(
              "inline-block size-1.5 rounded-full",
              running ? "live-pulse bg-[var(--color-bull)]" : "bg-[var(--color-dim)]",
            )}
          />
          {running
            ? `Writing ${fmtInt(TARGET_RATE)}/s in batches of ${BATCH_SIZE} (${MAX_IN_FLIGHT} in flight)`
            : "Subscribed"}
        </span>
        <button
          onClick={onToggle}
          disabled={loading}
          className={cn(
            "h-7 rounded-md px-3 text-[12px] font-medium transition-colors disabled:opacity-50",
            running
              ? "border border-[var(--color-line)] bg-secondary text-foreground hover:bg-[#1f232a]"
              : "bg-foreground text-background hover:bg-white",
          )}
        >
          {running ? "Stop ticker" : "Start ticker"}
        </button>
      </div>
    </header>
  );
});

function Kpi({ label, value, strong }: { label: string; value: ReactNode; strong?: boolean }) {
  return (
    <div className="flex flex-col leading-none">
      <span className="mb-1 text-[10px] text-[var(--color-dim)]">{label}</span>
      <span
        className={cn(
          "font-mono tabular-nums",
          strong ? "text-[15px] font-semibold text-[var(--color-accent-warm)]" : "text-[13px] text-foreground",
        )}
      >
        {value}
      </span>
    </div>
  );
}

function BrandMark() {
  return (
    <svg viewBox="0 0 48 64" width="14" height="19" fill="currentColor" aria-hidden className="text-[var(--color-accent-warm)]">
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Market table
// ---------------------------------------------------------------------------

const TABLE_COLS = "grid-cols-[18px_52px_64px_60px_minmax(0,1fr)_52px]";

const MarketTable = memo(function MarketTable({
  quotes,
  spark,
  end,
  selected,
  watchSet,
  loading,
  onSelect,
  onToggleWatch,
}: {
  quotes: Quote[];
  spark: Map<string, Candle[]>;
  end: number;
  selected: string;
  watchSet: Set<string>;
  loading: boolean;
  onSelect: (s: string) => void;
  onToggleWatch: (s: string) => void;
}) {
  // Watched symbols first; otherwise sector then symbol, so rows hold still.
  const rows = [...quotes].sort(
    (a, b) => Number(watchSet.has(b.symbol)) - Number(watchSet.has(a.symbol)),
  );
  const from = end - (SPARK_SECONDS - 1) * 1000;
  return (
    <Panel title="Market" meta={`${quotes.length} symbols`}>
      <div
        className={cn(
          "grid shrink-0 items-center gap-x-2 border-y border-[var(--color-line)] px-3 py-1.5 text-[10px] uppercase tracking-wide text-[var(--color-dim)]",
          TABLE_COLS,
        )}
      >
        <span />
        <span>Symbol</span>
        <span className="text-right">Last</span>
        <span className="text-right">Chg %</span>
        <span className="text-center">60 s</span>
        <span className="text-right">Volume</span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {loading && quotes.length === 0 ? (
          <div className="px-3 py-6 text-xs text-muted-foreground">Loading symbols...</div>
        ) : (
          rows.map((q) => (
            <MarketRow
              key={q.symbol}
              quote={q}
              closes={(spark.get(q.symbol) ?? EMPTY).filter((c) => c.t >= from).map((c) => c.close)}
              selected={selected === q.symbol}
              watched={watchSet.has(q.symbol)}
              onSelect={onSelect}
              onToggleWatch={onToggleWatch}
            />
          ))
        )}
      </div>
    </Panel>
  );
});

function MarketRow({
  quote: q,
  closes,
  selected,
  watched,
  onSelect,
  onToggleWatch,
}: {
  quote: Quote;
  closes: number[];
  selected: boolean;
  watched: boolean;
  onSelect: (s: string) => void;
  onToggleWatch: (s: string) => void;
}) {
  // Direction of the last price change, for the tick flash.
  const prev = useRef(q.price);
  const dir = useRef<"up" | "down" | null>(null);
  if (q.price !== prev.current) {
    dir.current = q.price > prev.current ? "up" : "down";
    prev.current = q.price;
  }
  return (
    <div
      onClick={() => onSelect(q.symbol)}
      className={cn(
        "grid h-[25px] cursor-pointer items-center gap-x-2 border-b border-[var(--color-grid)] px-3 text-[12px]",
        TABLE_COLS,
        selected
          ? "bg-[#161a20] shadow-[inset_2px_0_0_var(--color-accent-warm)]"
          : "hover:bg-[#12151a]",
      )}
    >
      <button
        onClick={(e) => {
          e.stopPropagation();
          onToggleWatch(q.symbol);
        }}
        title={watched ? "Remove from watchlist" : "Add to watchlist"}
        className={cn(
          "grid size-4 place-items-center",
          watched ? "text-[var(--color-accent-warm)]" : "text-[#3a404a] hover:text-muted-foreground",
        )}
      >
        <Star className={cn("size-3", watched && "fill-current")} />
      </button>
      <span className="font-medium">{q.symbol}</span>
      <span
        key={q.price}
        className={cn(
          "text-right font-mono tabular-nums",
          dir.current === "up" && "animate-[tick-up_500ms_ease-out]",
          dir.current === "down" && "animate-[tick-down_500ms_ease-out]",
        )}
      >
        {fmtPrice(q.price)}
      </span>
      <Up value={q.change} className="text-right tabular-nums">
        {fmtPct(q.pct)}
      </Up>
      <div className="flex justify-center">
        <Sparkline values={closes} up={q.change >= 0} width={92} height={18} />
      </div>
      <span className="text-right font-mono tabular-nums text-muted-foreground">{fmtCompact(q.volume)}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Breadth
// ---------------------------------------------------------------------------

const BreadthPanel = memo(function BreadthPanel({ breadth: b }: { breadth: Breadth }) {
  const total = Math.max(1, b.advancing + b.declining + b.unchanged);
  const volTotal = Math.max(1, b.upVolume + b.downVolume);
  const maxAbs = Math.max(0.25, ...b.sectors.map((s) => Math.abs(s.pct)));
  return (
    <Panel title="Breadth" meta="Change from open">
      <div className="flex flex-col gap-2.5 px-3 pb-2.5">
        <SplitBar
          leftLabel="Advancing"
          rightLabel="Declining"
          left={b.advancing}
          right={b.declining}
          middle={b.unchanged}
          total={total}
          format={fmtInt}
        />
        <SplitBar
          leftLabel="Up volume, 60 s"
          rightLabel="Down volume, 60 s"
          left={b.upVolume}
          right={b.downVolume}
          middle={0}
          total={volTotal}
          format={fmtCompact}
        />
        <div>
          <div className="grid grid-cols-[64px_40px_minmax(0,1fr)_56px] gap-x-2 pb-1 text-[10px] uppercase tracking-wide text-[var(--color-dim)]">
            <span>Sector</span>
            <span className="text-right">Up/Dn</span>
            <span />
            <span className="text-right">Chg %</span>
          </div>
          {b.sectors.map((s) => {
            const w = (Math.abs(s.pct) / maxAbs) * 50;
            return (
              <div key={s.name} className="grid h-[18px] grid-cols-[64px_40px_minmax(0,1fr)_56px] items-center gap-x-2 text-[12px]">
                <span className="text-foreground/90">{s.name}</span>
                <span className="text-right font-mono text-[11px] text-muted-foreground">
                  {s.up}/{s.down}
                </span>
                <div className="relative h-2">
                  <div className="absolute inset-y-0 left-1/2 w-px bg-[var(--color-line)]" />
                  <div
                    className="absolute inset-y-0 rounded-[2px]"
                    style={{
                      left: s.pct >= 0 ? "50%" : `${50 - w}%`,
                      width: `${w}%`,
                      background: s.pct >= 0 ? "var(--color-bull)" : "var(--color-bear)",
                      opacity: 0.85,
                    }}
                  />
                </div>
                <Up value={s.pct} className="text-right text-[11px] tabular-nums">
                  {fmtPct(s.pct)}
                </Up>
              </div>
            );
          })}
        </div>
      </div>
    </Panel>
  );
});

function SplitBar({
  leftLabel,
  rightLabel,
  left,
  right,
  middle,
  total,
  format,
}: {
  leftLabel: string;
  rightLabel: string;
  left: number;
  right: number;
  middle: number;
  total: number;
  format: (n: number) => string;
}) {
  return (
    <div>
      <div className="mb-1 flex justify-between text-[11px]">
        <span className="text-muted-foreground">
          {leftLabel} <span className="font-mono text-[var(--color-bull)]">{format(left)}</span>
        </span>
        <span className="text-muted-foreground">
          <span className="font-mono text-[var(--color-bear)]">{format(right)}</span> {rightLabel}
        </span>
      </div>
      <div className="flex h-2 gap-[2px]">
        <div className="rounded-[2px] bg-[var(--color-bull)] transition-[flex-grow] duration-300" style={{ flexGrow: left / total }} />
        {middle > 0 && (
          <div className="rounded-[2px] bg-[#3a404a] transition-[flex-grow] duration-300" style={{ flexGrow: middle / total }} />
        )}
        <div className="rounded-[2px] bg-[var(--color-bear)] transition-[flex-grow] duration-300" style={{ flexGrow: right / total }} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Selected symbol chart
// ---------------------------------------------------------------------------

const ChartPanel = memo(function ChartPanel({
  quote,
  candles,
  end,
}: {
  quote: Quote | null;
  candles: Candle[];
  end: number;
}) {
  let pv = 0;
  let vol = 0;
  let trades = 0;
  for (const c of candles) {
    pv += ((c.high + c.low + c.close) / 3) * c.volume;
    vol += c.volume;
    trades += c.trades;
  }
  const vwap = vol > 0 ? pv / vol : null;
  return (
    <section className="flex min-h-0 flex-col bg-[var(--color-panel)]">
      <div className="flex shrink-0 items-end gap-6 px-4 pb-3 pt-3">
        <div className="min-w-0">
          <div className="flex items-baseline gap-2">
            <span className="text-[18px] font-semibold tracking-tight">{quote?.symbol ?? "—"}</span>
            <span className="truncate text-[12px] text-muted-foreground">
              {quote ? `${quote.name} · ${quote.sector}` : ""}
            </span>
          </div>
          <div className="mt-0.5 flex items-baseline gap-3">
            <span className="font-mono text-[26px] font-medium leading-none tracking-tight tabular-nums">
              {quote ? fmtPrice(quote.price) : "—"}
            </span>
            {quote && (
              <Up value={quote.change} className="text-[13px] tabular-nums">
                {fmtSigned(quote.change)} ({fmtPct(quote.pct)})
              </Up>
            )}
          </div>
        </div>
        <div className="ml-auto grid grid-cols-6 gap-x-5">
          <ChartStat label="Open" value={quote ? fmtPrice(quote.openPrice) : "—"} />
          <ChartStat label="High" value={quote ? fmtPrice(quote.dayHigh) : "—"} />
          <ChartStat label="Low" value={quote ? fmtPrice(quote.dayLow) : "—"} />
          <ChartStat label={`VWAP ${CHART_SECONDS} s`} value={vwap === null ? "—" : fmtPrice(vwap)} />
          <ChartStat label={`Trades ${CHART_SECONDS} s`} value={fmtInt(trades)} />
          <ChartStat label="Volume" value={quote ? fmtCompact(quote.volume) : "—"} />
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-3 border-t border-[var(--color-line)] px-4 py-1.5 text-[11px] text-[var(--color-dim)]">
        <span>One-second candles, last {CHART_SECONDS} s</span>
        <span className="flex items-center gap-1">
          <span className="inline-block size-2 rounded-[2px] bg-[var(--color-bull)]" /> Close at or above open
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block size-2 rounded-[2px] bg-[var(--color-bear)]" /> Close below open
        </span>
        <span className="ml-auto">Volume below</span>
      </div>
      <div className="min-h-0 flex-1 px-3 pb-1 pt-2">
        <CandleChart
          candles={candles}
          end={end}
          seconds={CHART_SECONDS}
          last={quote?.price ?? null}
          reference={quote?.openPrice ?? null}
        />
      </div>
    </section>
  );
});

function ChartStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col items-end leading-none">
      <span className="mb-1 whitespace-nowrap text-[10px] text-[var(--color-dim)]">{label}</span>
      <span className="font-mono text-[13px] tabular-nums">{value}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Throughput
// ---------------------------------------------------------------------------

const ThroughputPanel = memo(function ThroughputPanel({
  series,
  end,
  trades,
  idle,
}: {
  series: number[];
  end: number;
  trades: number;
  idle: boolean;
}) {
  // The newest second is still filling, so stats use complete seconds.
  const complete = series.slice(0, -1);
  const active = complete.filter((v) => v > 0);
  const nowRate = idle ? 0 : complete[complete.length - 1] ?? 0;
  const peak = Math.max(0, ...complete);
  const avg = active.length ? active.reduce((a, b) => a + b, 0) / active.length : 0;
  return (
    <Panel title="Fills per second" meta="All symbols, from one-second candles">
      <div className="grid shrink-0 grid-cols-4 gap-px border-y border-[var(--color-line)] bg-[var(--color-line)]">
        <Tile label="Now" value={fmtInt(nowRate)} />
        <Tile label={`Peak ${CHART_SECONDS} s`} value={fmtInt(peak)} />
        <Tile label="Average, active" value={fmtInt(Math.round(avg))} />
        <Tile label="Trades recorded" value={fmtInt(trades)} />
      </div>
      <div className="min-h-0 flex-1 px-3 pb-2 pt-3">
        <ThroughputChart series={complete} end={(end || Date.now()) - 1000} />
      </div>
    </Panel>
  );
});

function Tile({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-[var(--color-panel)] px-3 py-2">
      <div className="text-[10px] text-[var(--color-dim)]">{label}</div>
      <div className="mt-0.5 font-mono text-[15px] tabular-nums">{value}</div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Gainers and losers
// ---------------------------------------------------------------------------

const MoversPanel = memo(function MoversPanel({
  quotes,
  onSelect,
}: {
  quotes: Quote[];
  onSelect: (s: string) => void;
}) {
  const byPct = [...quotes].sort((a, b) => b.pct - a.pct);
  const gainers = byPct.slice(0, 5);
  const losers = byPct.slice(-5).reverse();
  const maxAbs = Math.max(0.1, ...quotes.map((q) => Math.abs(q.pct)));
  const list = (title: string, rows: Quote[]) => (
    <div>
      <div className="px-3 pb-0.5 pt-1 text-[10px] uppercase tracking-wide text-[var(--color-dim)]">{title}</div>
      {rows.map((q) => (
        <button
          key={q.symbol}
          onClick={() => onSelect(q.symbol)}
          className="grid h-[18px] w-full grid-cols-[44px_52px_minmax(0,1fr)_54px] items-center gap-x-2 px-3 text-left text-[12px] hover:bg-[#12151a]"
        >
          <span className="font-medium">{q.symbol}</span>
          <span className="text-right font-mono tabular-nums text-muted-foreground">{fmtPrice(q.price)}</span>
          <div className="h-1.5">
            <div
              className="h-full rounded-[2px]"
              style={{
                width: `${(Math.abs(q.pct) / maxAbs) * 100}%`,
                background: q.pct >= 0 ? "var(--color-bull)" : "var(--color-bear)",
                opacity: 0.8,
              }}
            />
          </div>
          <Up value={q.change} className="text-right tabular-nums">
            {fmtPct(q.pct)}
          </Up>
        </button>
      ))}
    </div>
  );
  return (
    <Panel title="Movers" meta="Change from open">
      <div className="flex min-h-0 flex-col gap-1 border-t border-[var(--color-line)] pt-0.5">
        {list("Gainers", gainers)}
        {list("Losers", losers)}
      </div>
    </Panel>
  );
});

// ---------------------------------------------------------------------------
// Trade tape
// ---------------------------------------------------------------------------

const TAPE_COLS = "grid-cols-[92px_48px_36px_minmax(0,1fr)_64px]";

const TapePanel = memo(function TapePanel({ fills, lastSeq }: { fills: Fill[]; lastSeq: number }) {
  const rows = fills.filter((f) => f.seq > 0);
  return (
    <Panel title="Trade tape" meta={lastSeq > 0 ? `Fill #${fmtInt(lastSeq)}` : "No fills yet"}>
      <div
        className={cn(
          "grid shrink-0 items-center gap-x-2 border-y border-[var(--color-line)] px-3 py-1.5 text-[10px] uppercase tracking-wide text-[var(--color-dim)]",
          TAPE_COLS,
        )}
      >
        <span>Time</span>
        <span>Symbol</span>
        <span>Side</span>
        <span className="text-right">Size</span>
        <span className="text-right">Price</span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden">
        {rows.length === 0 ? (
          <div className="px-3 py-6 text-xs text-muted-foreground">Fills appear here when a tab runs the ticker.</div>
        ) : (
          rows.map((f) => {
            const buy = f.side === "buy";
            return (
              <div
                key={f.seq}
                className={cn(
                  "grid h-5 items-center gap-x-2 border-b border-[var(--color-grid)] px-3 text-[12px]",
                  TAPE_COLS,
                  buy ? "fill-in-buy" : "fill-in-sell",
                )}
              >
                <span className="font-mono text-[11px] tabular-nums text-[var(--color-dim)]">
                  {fmtClock(Date.parse(f.at), true)}
                </span>
                <span className="font-medium">{f.symbol}</span>
                <span className={buy ? "text-[var(--color-bull)]" : "text-[var(--color-bear)]"}>
                  {buy ? "Buy" : "Sell"}
                </span>
                <span className="text-right font-mono tabular-nums text-muted-foreground">{fmtInt(f.qty)}</span>
                <span className="text-right font-mono tabular-nums">{fmtPrice(f.price)}</span>
              </div>
            );
          })
        )}
      </div>
    </Panel>
  );
});
