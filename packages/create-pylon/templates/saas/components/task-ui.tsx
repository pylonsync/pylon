import React from "react";
import { initials } from "@/lib/names";
import { daysUntil, type TaskStatus } from "@/lib/tasks";

// Small presentational pieces shared by the Overview, the project list, and
// the task board. No data access here: every component renders from props.

/** Status glyph: an outlined circle that fills as the task moves to done. */
export function StatusIcon({
  status,
  className = "size-3.5",
}: {
  status: string;
  className?: string;
}) {
  const s = status as TaskStatus;
  if (s === "done") {
    return (
      <svg viewBox="0 0 14 14" className={`${className} shrink-0 text-brand`} aria-hidden>
        <circle cx="7" cy="7" r="6.25" fill="currentColor" stroke="currentColor" strokeWidth="1.5" />
        <path d="M4.3 7.2 6.1 9l3.6-3.8" fill="none" stroke="white" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }
  const tone =
    s === "in_progress" ? "text-amber-500" : s === "in_review" ? "text-emerald-500" : "text-zinc-400";
  // The share of the inner disc that is filled: 0 for to do, half for in
  // progress, three quarters for in review.
  const fill = s === "in_progress" ? 0.5 : s === "in_review" ? 0.75 : 0;
  const r = 3.25;
  const c = 2 * Math.PI * r;
  return (
    <svg viewBox="0 0 14 14" className={`${className} shrink-0 ${tone}`} aria-hidden>
      <circle cx="7" cy="7" r="6.25" fill="none" stroke="currentColor" strokeWidth="1.5" />
      {fill > 0 ? (
        <circle
          cx="7"
          cy="7"
          r={r}
          fill="none"
          stroke="currentColor"
          strokeWidth={r * 2}
          strokeDasharray={`${c * fill} ${c}`}
          transform="rotate(-90 7 7)"
        />
      ) : null}
    </svg>
  );
}

/** Priority glyph: three bars for high/medium/low, a filled square for urgent. */
export function PriorityIcon({
  priority,
  className = "size-3.5",
}: {
  priority?: string | null;
  className?: string;
}) {
  const p = priority ?? "none";
  if (p === "urgent") {
    return (
      <svg viewBox="0 0 14 14" className={`${className} shrink-0 text-orange-500`} aria-hidden>
        <rect x="1" y="1" width="12" height="12" rx="3" fill="currentColor" />
        <path d="M7 3.8v4" stroke="white" strokeWidth="1.6" strokeLinecap="round" />
        <circle cx="7" cy="10.1" r="0.95" fill="white" />
      </svg>
    );
  }
  if (p === "none") {
    return (
      <svg viewBox="0 0 14 14" className={`${className} shrink-0 text-zinc-300`} aria-hidden>
        <path d="M2.5 7h2M6 7h2M9.5 7h2" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
    );
  }
  const level = p === "high" ? 3 : p === "medium" ? 2 : 1;
  return (
    <svg viewBox="0 0 14 14" className={`${className} shrink-0 text-zinc-600`} aria-hidden>
      {[0, 1, 2].map((i) => (
        <rect
          key={i}
          x={2 + i * 3.75}
          y={9.5 - i * 3}
          width="2.5"
          height={2.5 + i * 3}
          rx="0.75"
          fill="currentColor"
          opacity={i < level ? 1 : 0.22}
        />
      ))}
    </svg>
  );
}

// Stable avatar color per person, hashed from their id.
const AVATAR_TONES = [
  "bg-[#e0e7ff] text-[#3730a3]",
  "bg-[#dcfce7] text-[#166534]",
  "bg-[#fef3c7] text-[#92400e]",
  "bg-[#fce7f3] text-[#9d174d]",
  "bg-[#e0f2fe] text-[#075985]",
  "bg-[#ede9fe] text-[#5b21b6]",
];

export function toneFor(key: string): string {
  let h = 0;
  for (let i = 0; i < key.length; i++) h = (h * 31 + key.charCodeAt(i)) >>> 0;
  return AVATAR_TONES[h % AVATAR_TONES.length];
}

export function Avatar({
  id,
  name,
  size = "sm",
}: {
  id: string;
  name: string;
  size?: "xs" | "sm" | "md";
}) {
  const dims =
    size === "xs" ? "size-[18px] text-[8.5px]" : size === "sm" ? "size-6 text-[10px]" : "size-8 text-[11.5px]";
  return (
    <span
      title={name}
      className={`inline-flex shrink-0 items-center justify-center rounded-full font-semibold ${dims} ${toneFor(id)}`}
    >
      {initials(name)}
    </span>
  );
}

/** A dashed ring for tasks with nobody assigned. */
export function EmptyAvatar({ size = "sm" }: { size?: "xs" | "sm" }) {
  const dims = size === "xs" ? "size-[18px]" : "size-6";
  return (
    <span
      title="Unassigned"
      className={`inline-flex shrink-0 rounded-full border border-dashed border-zinc-300 ${dims}`}
    />
  );
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "Sep 30" from `YYYY-MM-DD`, read as a calendar day (no time zone shift). */
export function shortDay(day: string): string {
  const [, m, d] = day.split("-").map(Number);
  return `${MONTHS[(m ?? 1) - 1]} ${d}`;
}

/**
 * Due date with its urgency: red when overdue, amber when due today or
 * tomorrow. `today` comes from the server render so the SSR HTML and the
 * hydrated client agree.
 */
export function DueLabel({
  due,
  today,
  done,
}: {
  due?: string | null;
  today: string;
  done?: boolean;
}) {
  if (!due) return null;
  const days = daysUntil(due, today);
  const tone = done
    ? "text-zinc-400"
    : days < 0
      ? "text-red-600"
      : days <= 1
        ? "text-amber-600"
        : "text-zinc-500";
  const label = done ? shortDay(due) : days === 0 ? "Today" : days === 1 ? "Tomorrow" : shortDay(due);
  return <span className={`whitespace-nowrap text-[12px] tabular-nums ${tone}`}>{label}</span>;
}

/** A thin progress bar with the percentage beside it. */
export function Progress({ percent, className = "" }: { percent: number; className?: string }) {
  return (
    <div className={`flex items-center gap-2.5 ${className}`}>
      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-zinc-100">
        <div className="h-full rounded-full bg-brand" style={{ width: `${percent}%` }} />
      </div>
      <span className="w-9 text-right font-mono text-[11.5px] tabular-nums text-zinc-500">{percent}%</span>
    </div>
  );
}

/** Project monogram tile, colored per project. */
export function ProjectMark({ id, name, className = "size-7 text-[11px]" }: { id: string; name: string; className?: string }) {
  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded-md font-semibold ${className} ${toneFor(id)}`}
    >
      {initials(name)}
    </span>
  );
}
