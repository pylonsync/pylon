// The pipeline model: stages, money, forecasting, and grouping.
//
// Everything here is pure — no React, no `db`. That's what makes the numbers on
// the board testable without standing up the app, and it's the layer to extend
// first when you change how the business measures itself.

export interface Stage {
  id: string;
  label: string;
  /** Terminal stages leave the forecast: a won or lost deal is no longer open. */
  closed?: "won" | "lost";
  /** Rough likelihood a deal at this stage closes, for the weighted forecast. */
  probability: number;
}

export const PIPELINE: Stage[] = [
  { id: "lead", label: "Lead", probability: 0.1 },
  { id: "qualified", label: "Qualified", probability: 0.3 },
  { id: "proposal", label: "Proposal", probability: 0.6 },
  { id: "won", label: "Won", closed: "won", probability: 1 },
  { id: "lost", label: "Lost", closed: "lost", probability: 0 },
];

/** Stages shown as board columns — closed/lost is history, not pipeline. */
export const BOARD_STAGES = PIPELINE.filter((s) => s.closed !== "lost");

export const ACTIVITY_KINDS = ["note", "call", "email", "meeting"] as const;
export type ActivityKind = (typeof ACTIVITY_KINDS)[number];

export interface Deal {
  id: string;
  title: string;
  value?: number | null;
  stage: string;
  companyId?: string | null;
  contactId?: string | null;
  closeDate?: string | null;
  ownerId?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export function stageById(id: string): Stage | undefined {
  return PIPELINE.find((s) => s.id === id);
}

export function isValidStage(id: string): boolean {
  return PIPELINE.some((s) => s.id === id);
}

export function isOpen(deal: { stage: string }): boolean {
  return !stageById(deal.stage)?.closed;
}

/** The stage a deal advances to, or null when it's already terminal. */
export function nextStage(current: string): string | null {
  const open = PIPELINE.filter((s) => s.closed !== "lost");
  const index = open.findIndex((s) => s.id === current);
  if (index === -1 || index === open.length - 1) return null;
  return open[index + 1].id;
}

export function previousStage(current: string): string | null {
  const open = PIPELINE.filter((s) => s.closed !== "lost");
  const index = open.findIndex((s) => s.id === current);
  if (index <= 0) return null;
  return open[index - 1].id;
}

export interface Column {
  stage: Stage;
  deals: Deal[];
  total: number;
}

/**
 * Group deals into board columns, newest first within a column. Deals with an
 * unrecognised stage are dropped rather than silently piled into "Lead" — a
 * stage typo should be visible in a test, not disguised as a lead.
 */
export function groupByStage(deals: Deal[], stages: Stage[] = BOARD_STAGES): Column[] {
  return stages.map((stage) => {
    const inStage = deals
      .filter((deal) => deal.stage === stage.id)
      .sort((a, b) => (b.createdAt ?? "").localeCompare(a.createdAt ?? ""));
    return { stage, deals: inStage, total: sumValue(inStage) };
  });
}

export function sumValue(deals: Deal[]): number {
  return deals.reduce((sum, deal) => sum + (Number(deal.value) || 0), 0);
}

export interface Metrics {
  /** Total value of every deal still in play. */
  open: number;
  /** Open value scaled by each stage's probability — the honest number. */
  weighted: number;
  won: number;
  lost: number;
  openCount: number;
  /** Won / (won + lost), or null when nothing has closed yet. */
  winRate: number | null;
}

export function metrics(deals: Deal[]): Metrics {
  let open = 0;
  let weighted = 0;
  let won = 0;
  let lost = 0;
  let openCount = 0;
  let wonCount = 0;
  let lostCount = 0;

  for (const deal of deals) {
    const value = Number(deal.value) || 0;
    const stage = stageById(deal.stage);
    if (!stage) continue;
    if (stage.closed === "won") {
      won += value;
      wonCount += 1;
    } else if (stage.closed === "lost") {
      lost += value;
      lostCount += 1;
    } else {
      open += value;
      weighted += value * stage.probability;
      openCount += 1;
    }
  }

  const decided = wonCount + lostCount;
  return {
    open,
    weighted: Math.round(weighted),
    won,
    lost,
    openCount,
    winRate: decided === 0 ? null : wonCount / decided,
  };
}

/**
 * Compact currency for dense UI: $1.2M, $840K, $1,200. Board columns and cards
 * have to line up in a narrow space, and "$1,240,000" wrecks that.
 */
export function money(value: number | null | undefined): string {
  const n = Number(value) || 0;
  const sign = n < 0 ? "-" : "";
  const abs = Math.abs(n);
  if (abs >= 1_000_000) return `${sign}$${trim(abs / 1_000_000)}M`;
  if (abs >= 10_000) return `${sign}$${Math.round(abs / 1_000)}K`;
  return `${sign}$${Math.round(abs).toLocaleString("en-US")}`;
}

function trim(value: number): string {
  return value.toFixed(1).replace(/\.0$/, "");
}

export function percent(value: number | null): string {
  return value === null ? "—" : `${Math.round(value * 100)}%`;
}

/** Initials for an avatar: "Acme Corp" → "AC", "jordan@x.com" → "J". */
export function initials(nameOrEmail: string | null | undefined): string {
  const raw = (nameOrEmail ?? "").trim();
  if (!raw) return "?";
  const name = raw.includes("@") ? raw.split("@")[0] : raw;
  const parts = name.split(/[\s._-]+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/**
 * A deterministic accent per record, so the same company keeps the same colour
 * across sessions and machines. Hashing beats storing a colour: no migration,
 * no drift between two clients rendering the same row.
 */
export function accentIndex(seed: string, buckets = 6): number {
  let hash = 0;
  for (let i = 0; i < seed.length; i += 1) {
    hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  }
  return Math.abs(hash) % buckets;
}

/** "just now" / "4h ago" / "Mar 3" — timeline density without a date library. */
export function relativeTime(
  iso: string | null | undefined,
  now: number = Date.now(),
): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return "";
  const seconds = Math.round((now - then) / 1000);
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(then).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

/** Days until close: negative is overdue, null when there's no date set. */
export function daysUntil(
  iso: string | null | undefined,
  now: number = Date.now(),
): number | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return null;
  const startOfDay = (ms: number) => {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
  return Math.round((startOfDay(then) - startOfDay(now)) / 86_400_000);
}
