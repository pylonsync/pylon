const intFmt = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });
const compactFmt = new Intl.NumberFormat("en-US", {
  notation: "compact",
  maximumFractionDigits: 1,
});

export const fmtPrice = (n: number) => n.toFixed(2);
export const fmtInt = (n: number) => intFmt.format(n);
export const fmtCompact = (n: number) => compactFmt.format(n);
export const fmtSigned = (n: number, digits = 2) =>
  `${n > 0 ? "+" : n < 0 ? "−" : ""}${Math.abs(n).toFixed(digits)}`;
export const fmtPct = (n: number) => `${fmtSigned(n)}%`;

export function fmtClock(ms: number, withMs = false) {
  const d = new Date(ms);
  const p = (v: number, w = 2) => String(v).padStart(w, "0");
  const base = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  return withMs ? `${base}.${p(d.getMilliseconds(), 3)}` : base;
}

/** Percentile of an unsorted sample. Returns null for an empty sample. */
export function percentile(sample: number[], p: number): number | null {
  if (sample.length === 0) return null;
  const sorted = [...sample].sort((a, b) => a - b);
  const i = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[i];
}

/** Round step for about `count` axis ticks across `span`. */
export function niceStep(span: number, count: number) {
  const raw = span / Math.max(1, count);
  const mag = 10 ** Math.floor(Math.log10(raw));
  const norm = raw / mag;
  const nice = norm < 1.5 ? 1 : norm < 3 ? 2 : norm < 7 ? 5 : 10;
  return nice * mag;
}
