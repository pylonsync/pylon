/**
 * Pylon Trade — live market dashboard.
 *
 * Layout:
 *   - Topbar: ticks/sec, active symbols, watchlist size, "Start ticker"
 *   - Watchlist column: symbols the user is tracking
 *   - Main: top movers table (sorted by % change)
 *   - Detail column: selected symbol metrics + sparkline
 *
 * One tab should click "Start ticker" — it generates trades for all
 * symbols at ~100 Hz. Every other tab passively subscribes to the
 * Ticker table and sees prices update live.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import {
  init,
  db,
  callFn,
  configureClient,
  storageKey,
} from "@pylonsync/react";
import { ArrowDownRight, ArrowUpRight, Star } from "lucide-react";
import { Button } from "@pylonsync/example-ui/button";
import { Badge } from "@pylonsync/example-ui/badge";
import { Separator } from "@pylonsync/example-ui/separator";
import { cn } from "@pylonsync/example-ui/utils";

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
  const { data: watches } = db.useQuery<Watch>(
    "Watch",
    userId ? { where: { userId } } : undefined,
  );

  const { data: recentTrades } = db.useQuery<Trade>(
    "Trade",
    selected ? { where: { symbol: selected } } : undefined,
  );

  const tickCount = useRef(0);

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
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!running || !tickers || tickers.length === 0) return;
    let cancelled = false;

    const step = async () => {
      if (cancelled) return;
      const batch: Promise<unknown>[] = [];
      for (let i = 0; i < 20; i++) {
        const t = tickers[Math.floor(Math.random() * tickers.length)];
        const move = (Math.random() - 0.5) * t.price * 0.002;
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
    return () => {
      cancelled = true;
    };
  }, [running, tickers]);

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
    <div className="grid h-screen grid-rows-[56px_1fr]">
      <Topbar
        ticksPerSec={ticksPerSec}
        symbols={(tickers ?? []).length}
        watchCount={(watches ?? []).length}
        running={running}
        onToggleRunning={() => setRunning((r) => !r)}
      />
      <div className="grid grid-cols-[260px_1fr_320px] overflow-hidden">
        <Watchlist
          watches={watches ?? []}
          tickers={tickers ?? []}
          selected={selected}
          onSelect={setSelected}
        />
        <Movers
          rows={sorted}
          selected={selected}
          watchSet={watchSet}
          onSelect={setSelected}
          onToggleWatch={toggleWatch}
        />
        <Detail ticker={selectedTicker} trades={recentTrades ?? []} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Topbar
// ---------------------------------------------------------------------------

function Topbar({
  ticksPerSec,
  symbols,
  watchCount,
  running,
  onToggleRunning,
}: {
  ticksPerSec: number;
  symbols: number;
  watchCount: number;
  running: boolean;
  onToggleRunning: () => void;
}) {
  return (
    <header className="flex items-center gap-8 border-b bg-background px-5">
      <div className="flex items-center gap-2.5 font-mono text-[13px] font-medium text-foreground">
        <BrandMark />
        <span>Pylon · Trade</span>
      </div>
      <div className="flex items-center gap-6">
        <Stat label="TICKS/S" value={ticksPerSec} />
        <Stat label="SYMBOLS" value={symbols} />
        <Stat label="WATCH" value={watchCount} />
      </div>
      <div className="ml-auto">
        <Button
          variant={running ? "secondary" : "default"}
          size="sm"
          onClick={onToggleRunning}
        >
          {running ? "Stop ticker" : "Start ticker"}
        </Button>
      </div>
    </header>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </span>
      <span className="font-mono text-sm tabular-nums text-foreground">
        {value.toLocaleString()}
      </span>
    </div>
  );
}

function BrandMark() {
  return (
    <svg
      viewBox="0 0 48 64"
      width="18"
      height="24"
      fill="currentColor"
      aria-hidden
      className="text-primary"
    >
      <path d="M24 2 L10 20 L24 32 Z" />
      <path d="M24 2 L38 20 L24 32 Z" />
      <path d="M24 32 L18 48 L24 62 L30 48 Z" />
      <path d="M6 30 Q3 46 16 56 L18 50 Q10 44 11 32 Z" />
      <path d="M42 30 Q45 46 32 56 L30 50 Q38 44 37 32 Z" />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Watchlist
// ---------------------------------------------------------------------------

function Watchlist({
  watches,
  tickers,
  selected,
  onSelect,
}: {
  watches: Watch[];
  tickers: Ticker[];
  selected: string | null;
  onSelect: (s: string) => void;
}) {
  return (
    <aside className="flex flex-col overflow-hidden border-r bg-card">
      <ColHead>Watchlist</ColHead>
      <div className="flex-1 overflow-y-auto">
        {watches.length === 0 ? (
          <Empty>Click a row → ★ to watch.</Empty>
        ) : (
          watches.map((w) => {
            const t = tickers.find((tt) => tt.symbol === w.symbol);
            if (!t) return null;
            const pct =
              t.openPrice > 0 ? ((t.price - t.openPrice) / t.openPrice) * 100 : 0;
            const up = pct >= 0;
            return (
              <button
                key={w.id}
                onClick={() => onSelect(t.symbol)}
                className={cn(
                  "flex w-full items-center gap-3 border-l-2 px-4 py-2 text-left text-sm transition-colors",
                  selected === t.symbol
                    ? "border-primary bg-accent"
                    : "border-transparent hover:bg-muted/40",
                )}
              >
                <span className="font-mono font-medium">{t.symbol}</span>
                <span className="ml-auto font-mono tabular-nums">
                  {t.price.toFixed(2)}
                </span>
                <span
                  className={cn(
                    "font-mono text-xs tabular-nums",
                    up ? "text-[var(--color-bull)]" : "text-[var(--color-bear)]",
                  )}
                >
                  {up ? "+" : ""}
                  {pct.toFixed(2)}%
                </span>
              </button>
            );
          })
        )}
      </div>
    </aside>
  );
}

// ---------------------------------------------------------------------------
// Movers table
// ---------------------------------------------------------------------------

function Movers({
  rows,
  selected,
  watchSet,
  onSelect,
  onToggleWatch,
}: {
  rows: Array<Ticker & { pct: number }>;
  selected: string | null;
  watchSet: Set<string>;
  onSelect: (s: string) => void;
  onToggleWatch: (s: string) => void;
}) {
  return (
    <main className="flex flex-col overflow-hidden">
      <ColHead>Top movers</ColHead>
      <div className="flex-1 overflow-y-auto">
        <div className="grid grid-cols-[80px_minmax(160px,1fr)_120px_100px_100px_120px_40px] items-center gap-2 border-b px-4 py-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
          <span>Symbol</span>
          <span>Name</span>
          <span>Sector</span>
          <span className="text-right">Price</span>
          <span className="text-right">Day ±</span>
          <span className="text-right">Volume</span>
          <span />
        </div>
        {rows.map((t) => {
          const up = t.pct >= 0;
          const isSelected = selected === t.symbol;
          return (
            <div
              key={t.id}
              onClick={() => onSelect(t.symbol)}
              className={cn(
                "grid grid-cols-[80px_minmax(160px,1fr)_120px_100px_100px_120px_40px] cursor-pointer items-center gap-2 border-b border-border/40 px-4 py-2 text-sm transition-colors",
                isSelected ? "bg-accent" : "hover:bg-muted/30",
              )}
            >
              <span className="font-mono font-medium">{t.symbol}</span>
              <span className="truncate text-foreground/90">{t.name}</span>
              <span className="text-xs text-muted-foreground">{t.sector}</span>
              <span className="text-right font-mono tabular-nums">
                ${t.price.toFixed(2)}
              </span>
              <span
                className={cn(
                  "flex items-center justify-end gap-0.5 font-mono text-xs tabular-nums",
                  up ? "text-[var(--color-bull)]" : "text-[var(--color-bear)]",
                )}
              >
                {up ? (
                  <ArrowUpRight className="size-3" />
                ) : (
                  <ArrowDownRight className="size-3" />
                )}
                {up ? "+" : ""}
                {t.pct.toFixed(2)}%
              </span>
              <span className="text-right font-mono text-xs tabular-nums text-muted-foreground">
                {t.volume.toLocaleString()}
              </span>
              <button
                className={cn(
                  "flex size-7 items-center justify-center rounded text-muted-foreground transition-colors hover:text-foreground",
                  watchSet.has(t.symbol) && "text-amber-400",
                )}
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleWatch(t.symbol);
                }}
                title={watchSet.has(t.symbol) ? "Unwatch" : "Watch"}
              >
                <Star
                  className={cn("size-4", watchSet.has(t.symbol) && "fill-current")}
                />
              </button>
            </div>
          );
        })}
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

function Detail({
  ticker,
  trades,
}: {
  ticker: Ticker | null | undefined;
  trades: Trade[];
}) {
  return (
    <aside className="flex flex-col overflow-hidden border-l bg-card">
      <ColHead>{ticker ? ticker.symbol : "—"}</ColHead>
      {ticker ? (
        <div className="flex flex-col gap-4 p-5">
          <div className="text-sm text-muted-foreground">{ticker.name}</div>
          <div className="font-mono text-3xl font-semibold tabular-nums">
            ${ticker.price.toFixed(2)}
          </div>
          <Separator />
          <div className="flex flex-col gap-2 text-sm">
            <Row label="OPEN" value={`$${ticker.openPrice.toFixed(2)}`} />
            <Row label="HIGH" value={`$${ticker.dayHigh.toFixed(2)}`} />
            <Row label="LOW" value={`$${ticker.dayLow.toFixed(2)}`} />
            <Row label="VOL" value={ticker.volume.toLocaleString()} />
          </div>
          <Sparkline trades={trades} />
          <Badge variant="outline" className="self-start font-mono text-[10px]">
            Live
          </Badge>
        </div>
      ) : (
        <Empty>Pick a symbol.</Empty>
      )}
    </aside>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
        {label}
      </span>
      <span className="font-mono tabular-nums">{value}</span>
    </div>
  );
}

function Sparkline({ trades }: { trades: Trade[] }) {
  if (trades.length < 2) {
    return (
      <div className="flex h-20 items-center justify-center rounded border border-dashed text-xs text-muted-foreground">
        no trades yet
      </div>
    );
  }
  const sorted = [...trades]
    .sort((a, b) => +new Date(a.at) - +new Date(b.at))
    .slice(-60);
  const prices = sorted.map((t) => t.price);
  const min = Math.min(...prices);
  const max = Math.max(...prices);
  const span = Math.max(0.01, max - min);
  const W = 240;
  const H = 80;
  const points = prices
    .map((p, i) => {
      const x = (i / (prices.length - 1)) * W;
      const y = H - ((p - min) / span) * H;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  const trending = prices[prices.length - 1] >= prices[0];
  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      preserveAspectRatio="none"
      className={cn(
        "h-20 w-full",
        trending ? "text-[var(--color-bull)]" : "text-[var(--color-bear)]",
      )}
    >
      <polyline fill="none" stroke="currentColor" strokeWidth="1.5" points={points} />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function ColHead({ children }: { children: React.ReactNode }) {
  return (
    <div className="border-b px-4 py-2.5 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
      {children}
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-4 py-8 text-center text-xs text-muted-foreground">
      {children}
    </div>
  );
}
