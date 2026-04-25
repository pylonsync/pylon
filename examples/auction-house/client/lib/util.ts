export function formatCents(cents: number): string {
  if (cents >= 100_000) {
    return `$${(cents / 100).toLocaleString("en-US", {
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    })}`;
  }
  return `$${(cents / 100).toFixed(2)}`;
}

export function navigate(href: string) {
  if (window.location.hash === href) return;
  window.location.hash = href;
}

export function timeLeft(toIso: string | null | undefined): {
  ms: number;
  label: string;
} {
  if (!toIso) return { ms: 0, label: "—" };
  const ms = new Date(toIso).getTime() - Date.now();
  if (ms <= 0) return { ms: 0, label: "ended" };
  const s = Math.floor(ms / 1000);
  if (s < 60) return { ms, label: `${s}s` };
  const m = Math.floor(s / 60);
  if (m < 60) return { ms, label: `${m}m ${s % 60}s` };
  const h = Math.floor(m / 60);
  if (h < 24) return { ms, label: `${h}h ${m % 60}m` };
  const d = Math.floor(h / 24);
  return { ms, label: `${d}d ${h % 24}h` };
}

export function initials(name: string): string {
  return (
    name
      .split(/\s|@/)
      .filter(Boolean)
      .slice(0, 2)
      .map((p) => p[0]?.toUpperCase() ?? "")
      .join("") || "?"
  );
}
