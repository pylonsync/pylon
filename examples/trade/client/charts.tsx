"use client";

import { memo, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { Candle, Quote } from "./types";
import { fmtClock, fmtCompact, fmtInt, fmtPct, fmtPrice, niceStep } from "./format";

/** Pixel size of an element, updated on resize. */
export function useSize<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize({ width: Math.floor(width), height: Math.floor(height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, size] as const;
}

// ---------------------------------------------------------------------------
// Sparkline: one symbol's one-second closes.
// ---------------------------------------------------------------------------

export const Sparkline = memo(function Sparkline({
  values,
  up,
  width = 84,
  height = 18,
}: {
  values: number[];
  up: boolean;
  width?: number;
  height?: number;
}) {
  if (values.length < 2) {
    return <svg width={width} height={height} aria-hidden />;
  }
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const pad = 2;
  const step = (width - pad * 2) / (values.length - 1);
  const pts = values.map(
    (v, i) => `${(pad + i * step).toFixed(1)},${(pad + (1 - (v - min) / span) * (height - pad * 2)).toFixed(1)}`,
  );
  const color = up ? "var(--color-bull)" : "var(--color-bear)";
  return (
    <svg width={width} height={height} aria-hidden className="block">
      <polygon
        points={`${pad},${height} ${pts.join(" ")} ${width - pad},${height}`}
        fill={color}
        opacity={0.1}
      />
      <polyline points={pts.join(" ")} fill="none" stroke={color} strokeWidth={1.25} strokeLinejoin="round" />
    </svg>
  );
});

// ---------------------------------------------------------------------------
// Candles with volume, one bar per second.
// ---------------------------------------------------------------------------

const AXIS_W = 58;
const TIME_H = 20;
const VOL_H = 64;
const GAP_H = 10;

export const CandleChart = memo(function CandleChart({
  candles,
  end,
  seconds,
  last,
  reference,
}: {
  candles: Candle[];
  /** Epoch ms of the newest second on the x axis. */
  end: number;
  seconds: number;
  last: number | null;
  /** Price the last-price tag is colored against (the open). */
  reference: number | null;
}) {
  const [ref, { width, height }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);

  const plotW = Math.max(0, width - AXIS_W);
  const priceH = Math.max(0, height - TIME_H - VOL_H - GAP_H);
  const slot = plotW / seconds;
  const start = end - (seconds - 1) * 1000;
  const xOf = (t: number) => ((t - start) / 1000) * slot + slot / 2;

  let lo = Infinity;
  let hi = -Infinity;
  let maxVol = 0;
  for (const c of candles) {
    lo = Math.min(lo, c.low);
    hi = Math.max(hi, c.high);
    maxVol = Math.max(maxVol, c.volume);
  }
  if (last !== null) {
    lo = Math.min(lo, last);
    hi = Math.max(hi, last);
  }
  const hasData = candles.length > 0 && Number.isFinite(lo);
  const padP = hasData ? Math.max((hi - lo) * 0.08, hi * 0.0005) : 1;
  const pLo = hasData ? lo - padP : 0;
  const pHi = hasData ? hi + padP : 1;
  const yOf = (p: number) => ((pHi - p) / (pHi - pLo)) * priceH;
  const volTop = priceH + GAP_H;
  const vOf = (v: number) => (maxVol > 0 ? (v / maxVol) * (VOL_H - 4) : 0);

  const step = niceStep(pHi - pLo, 6);
  const ticks: number[] = [];
  for (let p = Math.ceil(pLo / step) * step; p <= pHi; p += step) ticks.push(p);
  const timeTicks: number[] = [];
  const firstTick = Math.ceil(start / 15_000) * 15_000;
  for (let t = firstTick; t <= end; t += 15_000) timeTicks.push(t);

  const bodyW = Math.max(1, Math.min(9, slot * 0.64));
  const hovered = hover !== null ? candles.find((c) => c.t === hover) ?? null : null;
  const lastUp = last !== null && reference !== null ? last >= reference : true;

  function onMove(e: React.MouseEvent<SVGRectElement>) {
    const box = e.currentTarget.getBoundingClientRect();
    const i = Math.floor((e.clientX - box.left) / slot);
    const t = start + Math.max(0, Math.min(seconds - 1, i)) * 1000;
    setHover(t);
  }

  return (
    <div ref={ref} className="relative h-full w-full">
      {width > 0 && (
        <svg width={width} height={height} className="block" role="img" aria-label="One-second candles with volume">
          {ticks.map((p) => (
            <g key={p}>
              <line x1={0} x2={plotW} y1={yOf(p)} y2={yOf(p)} stroke="var(--color-grid)" />
              <text x={plotW + 8} y={yOf(p) + 3.5} className="fill-[var(--color-dim)] font-mono text-[10px]">
                {fmtPrice(p)}
              </text>
            </g>
          ))}
          {timeTicks.map((t) => (
            <g key={t}>
              <line x1={xOf(t)} x2={xOf(t)} y1={0} y2={volTop + VOL_H} stroke="var(--color-grid)" />
              <text
                x={Math.max(26, Math.min(plotW - 26, xOf(t)))}
                y={height - 6}
                textAnchor="middle"
                className="fill-[var(--color-dim)] font-mono text-[10px]"
              >
                {fmtClock(t)}
              </text>
            </g>
          ))}
          <line x1={0} x2={plotW} y1={volTop + VOL_H} y2={volTop + VOL_H} stroke="var(--color-line)" />
          <line x1={plotW} x2={plotW} y1={0} y2={volTop + VOL_H} stroke="var(--color-line)" />

          {candles.map((c) => {
            const x = xOf(c.t);
            if (x < 0 || x > plotW) return null;
            const up = c.close >= c.open;
            const color = up ? "var(--color-bull)" : "var(--color-bear)";
            const yO = yOf(c.open);
            const yC = yOf(c.close);
            const vh = vOf(c.volume);
            return (
              <g key={c.t}>
                <line x1={x} x2={x} y1={yOf(c.high)} y2={yOf(c.low)} stroke={color} strokeWidth={1} />
                <rect
                  x={x - bodyW / 2}
                  y={Math.min(yO, yC)}
                  width={bodyW}
                  height={Math.max(1, Math.abs(yC - yO))}
                  fill={color}
                  rx={1}
                />
                <rect
                  x={x - bodyW / 2}
                  y={volTop + VOL_H - vh}
                  width={bodyW}
                  height={vh}
                  fill={color}
                  opacity={0.38}
                  rx={1}
                />
              </g>
            );
          })}

          {last !== null && hasData && (
            <g>
              <line
                x1={0}
                x2={plotW}
                y1={yOf(last)}
                y2={yOf(last)}
                stroke={lastUp ? "var(--color-bull)" : "var(--color-bear)"}
                strokeDasharray="2 3"
                opacity={0.7}
              />
              <rect
                x={plotW + 1}
                y={yOf(last) - 9}
                width={AXIS_W - 2}
                height={18}
                rx={3}
                fill={lastUp ? "var(--color-bull)" : "var(--color-bear)"}
              />
              <text
                x={plotW + 8}
                y={yOf(last) + 4}
                className="fill-[#08090b] font-mono text-[11px] font-semibold"
              >
                {fmtPrice(last)}
              </text>
            </g>
          )}

          {hovered && (
            <line
              x1={xOf(hovered.t)}
              x2={xOf(hovered.t)}
              y1={0}
              y2={volTop + VOL_H}
              stroke="var(--color-muted-foreground)"
              strokeDasharray="3 3"
              opacity={0.6}
            />
          )}
          <rect
            x={0}
            y={0}
            width={plotW}
            height={volTop + VOL_H}
            fill="transparent"
            onMouseMove={onMove}
            onMouseLeave={() => setHover(null)}
          />
        </svg>
      )}
      {!hasData && width > 0 && (
        <div className="absolute inset-0 grid place-items-center text-xs text-muted-foreground">
          No trades in the last {seconds} seconds. Start the ticker to stream fills.
        </div>
      )}
      {hovered && (
        <Tooltip x={Math.min(xOf(hovered.t) + 12, plotW - 168)} y={8}>
          <div className="mb-1 font-mono text-[10px] text-muted-foreground">{fmtClock(hovered.t)}</div>
          <TipRow label="Open" value={fmtPrice(hovered.open)} />
          <TipRow label="High" value={fmtPrice(hovered.high)} />
          <TipRow label="Low" value={fmtPrice(hovered.low)} />
          <TipRow label="Close" value={fmtPrice(hovered.close)} />
          <TipRow label="Volume" value={fmtInt(hovered.volume)} />
          <TipRow label="Trades" value={fmtInt(hovered.trades)} />
        </Tooltip>
      )}
    </div>
  );
});

function Tooltip({ x, y, children }: { x: number; y: number; children: ReactNode }) {
  return (
    <div
      className="pointer-events-none absolute w-40 rounded-md border border-[var(--color-line)] bg-popover/95 px-2.5 py-2 shadow-xl shadow-black/40"
      style={{ left: Math.max(4, x), top: y }}
    >
      {children}
    </div>
  );
}

function TipRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between text-[11px] leading-5">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-mono text-foreground">{value}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Throughput: fills per second across the market.
// ---------------------------------------------------------------------------

export const ThroughputChart = memo(function ThroughputChart({
  series,
  end,
}: {
  /** Fills per second, oldest first, one entry per second ending at `end`. */
  series: number[];
  end: number;
}) {
  const [ref, { width, height }] = useSize<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const axisW = 40;
  const timeH = 16;
  // Room above the top gridline for its label.
  const padTop = 6;
  const plotW = Math.max(0, width - axisW);
  const plotH = Math.max(0, height - timeH - padTop);
  // Headroom above the peak so a steady rate does not sit on the frame.
  const max = Math.max(500, ...series) * 1.2;
  const step = niceStep(max, 3);
  const top = Math.ceil(max / step) * step;
  const slot = plotW / series.length;
  const ticks: number[] = [];
  for (let v = 0; v <= top; v += step) ticks.push(v);
  const n = series.length;
  const tAt = (i: number) => end - (n - 1 - i) * 1000;

  return (
    <div ref={ref} className="relative h-full w-full">
      {width > 0 && (
        <svg width={width} height={height} className="block" role="img" aria-label="Fills per second">
          <g transform={`translate(0 ${padTop})`}>
          {ticks.map((v) => {
            const y = plotH - (v / top) * plotH;
            return (
              <g key={v}>
                <line x1={0} x2={plotW} y1={y} y2={y} stroke="var(--color-grid)" />
                <text x={plotW + 6} y={y + 3.5} className="fill-[var(--color-dim)] font-mono text-[10px]">
                  {fmtCompact(v)}
                </text>
              </g>
            );
          })}
          <path
            d={`M0,${plotH} ${series
              .map((v, i) => `L${(i * slot + slot / 2).toFixed(1)},${(plotH - (v / top) * plotH).toFixed(1)}`)
              .join(" ")} L${plotW},${plotH} Z`}
            fill="var(--color-series)"
            opacity={0.14}
          />
          <polyline
            points={series
              .map((v, i) => `${(i * slot + slot / 2).toFixed(1)},${(plotH - (v / top) * plotH).toFixed(1)}`)
              .join(" ")}
            fill="none"
            stroke="var(--color-series)"
            strokeWidth={2}
            strokeLinejoin="round"
          />
          {hover !== null && (
            <circle
              cx={hover * slot + slot / 2}
              cy={plotH - (series[hover] / top) * plotH}
              r={4}
              fill="var(--color-series)"
              stroke="var(--color-panel)"
              strokeWidth={2}
            />
          )}
          {[0, Math.floor(n / 2), n - 1].map((i) => (
            <text
              key={i}
              x={Math.min(plotW - 2, Math.max(2, i * slot + slot / 2))}
              y={height - padTop - 3}
              textAnchor={i === 0 ? "start" : i === n - 1 ? "end" : "middle"}
              className="fill-[var(--color-dim)] font-mono text-[10px]"
            >
              {fmtClock(tAt(i))}
            </text>
          ))}
          <rect
            x={0}
            y={0}
            width={plotW}
            height={plotH}
            fill="transparent"
            onMouseMove={(e) => {
              const box = e.currentTarget.getBoundingClientRect();
              setHover(Math.max(0, Math.min(n - 1, Math.floor((e.clientX - box.left) / slot))));
            }}
            onMouseLeave={() => setHover(null)}
          />
          </g>
        </svg>
      )}
      {hover !== null && (
        <Tooltip x={Math.min(hover * slot + 10, plotW - 168)} y={4}>
          <TipRow label={fmtClock(tAt(hover))} value={`${fmtInt(series[hover])} fills`} />
        </Tooltip>
      )}
    </div>
  );
});

// ---------------------------------------------------------------------------
// Sector treemap: area = dollar volume, color = change from the open.
// ---------------------------------------------------------------------------

type Rect = { x: number; y: number; w: number; h: number };

/** Squarified treemap layout (Bruls, Huizing, van Wijk). */
function squarify<T>(items: Array<{ value: number; item: T }>, rect: Rect): Array<Rect & { item: T }> {
  const out: Array<Rect & { item: T }> = [];
  const total = items.reduce((a, b) => a + b.value, 0);
  if (total <= 0 || rect.w <= 0 || rect.h <= 0) return out;
  const scale = (rect.w * rect.h) / total;
  const nodes = items
    .filter((i) => i.value > 0)
    .map((i) => ({ area: i.value * scale, item: i.item }))
    .sort((a, b) => b.area - a.area);
  let r = { ...rect };
  let row: typeof nodes = [];

  const worst = (list: typeof nodes, side: number) => {
    const s = list.reduce((a, b) => a + b.area, 0);
    let mx = 0;
    let mn = Infinity;
    for (const n of list) {
      mx = Math.max(mx, n.area);
      mn = Math.min(mn, n.area);
    }
    return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn));
  };

  const place = (list: typeof nodes) => {
    const s = list.reduce((a, b) => a + b.area, 0);
    if (r.w >= r.h) {
      const colW = s / r.h;
      let y = r.y;
      for (const n of list) {
        const h = n.area / colW;
        out.push({ x: r.x, y, w: colW, h, item: n.item });
        y += h;
      }
      r = { x: r.x + colW, y: r.y, w: r.w - colW, h: r.h };
    } else {
      const rowH = s / r.w;
      let x = r.x;
      for (const n of list) {
        const w = n.area / rowH;
        out.push({ x, y: r.y, w, h: rowH, item: n.item });
        x += w;
      }
      r = { x: r.x, y: r.y + rowH, w: r.w, h: r.h - rowH };
    }
  };

  let i = 0;
  while (i < nodes.length) {
    const side = Math.min(r.w, r.h);
    const next = nodes[i];
    if (row.length === 0 || worst([...row, next], side) <= worst(row, side)) {
      row.push(next);
      i++;
    } else {
      place(row);
      row = [];
    }
  }
  if (row.length) place(row);
  return out;
}

/** Percent change that maps to full color. */
const HEAT_CLAMP = 2.5;

function heat(pct: number) {
  const k = Math.round(Math.min(1, Math.abs(pct) / HEAT_CLAMP) * 100);
  const pole = pct >= 0 ? "var(--color-heat-up)" : "var(--color-heat-down)";
  return `color-mix(in oklab, ${pole} ${k}%, var(--color-heat-mid))`;
}

export const SectorTreemap = memo(function SectorTreemap({
  quotes,
  selected,
  onSelect,
}: {
  quotes: Quote[];
  selected: string | null;
  onSelect: (symbol: string) => void;
}) {
  const [ref, { width, height }] = useSize<HTMLDivElement>();
  const bySector = new Map<string, Quote[]>();
  for (const q of quotes) {
    const list = bySector.get(q.sector) ?? [];
    list.push(q);
    bySector.set(q.sector, list);
  }
  const dollar = (q: Quote) => Math.max(1, q.volume * q.price);
  const sectors = squarify(
    [...bySector].map(([name, list]) => ({
      value: list.reduce((a, q) => a + dollar(q), 0),
      item: { name, list },
    })),
    { x: 0, y: 0, w: width, h: height },
  );
  const LABEL_H = 16;

  return (
    <div ref={ref} className="relative h-full w-full overflow-hidden">
      {sectors.map((s) => {
        const pct =
          s.item.list.reduce((a, q) => a + q.pct * dollar(q), 0) /
          s.item.list.reduce((a, q) => a + dollar(q), 0);
        const tiles = squarify(
          s.item.list.map((q) => ({ value: dollar(q), item: q })),
          { x: 0, y: 0, w: s.w - 2, h: s.h - LABEL_H - 2 },
        );
        return (
          <div
            key={s.item.name}
            className="absolute"
            style={{ left: s.x + 1, top: s.y + 1, width: s.w - 2, height: s.h - 2 }}
          >
            <div className="flex h-4 items-center justify-between px-1 text-[10px] leading-none">
              <span className="font-medium text-muted-foreground">{s.item.name}</span>
              <span className="font-mono text-[var(--color-dim)]">{fmtPct(pct)}</span>
            </div>
            {tiles.map((t) => {
              const q = t.item;
              const big = t.w > 54 && t.h > 34;
              const fits = t.w > 30 && t.h > 16;
              return (
                <button
                  key={q.symbol}
                  onClick={() => onSelect(q.symbol)}
                  title={`${q.symbol} ${q.name}: ${fmtPrice(q.price)} (${fmtPct(q.pct)}), $${fmtCompact(dollar(q))} traded`}
                  className="absolute flex flex-col items-center justify-center overflow-hidden rounded-[3px] text-center transition-[background-color] duration-300"
                  style={{
                    left: t.x + 1,
                    top: LABEL_H + t.y + 1,
                    width: Math.max(0, t.w - 2),
                    height: Math.max(0, t.h - 2),
                    background: heat(q.pct),
                    boxShadow: selected === q.symbol ? "inset 0 0 0 1.5px var(--color-accent-warm)" : undefined,
                  }}
                >
                  {fits && (
                    <span className={big ? "text-[12px] font-semibold text-white" : "text-[10px] font-semibold text-white"}>
                      {q.symbol}
                    </span>
                  )}
                  {big && <span className="font-mono text-[10px] text-white/80">{fmtPct(q.pct)}</span>}
                </button>
              );
            })}
          </div>
        );
      })}
    </div>
  );
});
