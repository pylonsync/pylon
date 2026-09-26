"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Download, LogOut, Search } from "lucide-react";
import type { WaitlistStatsData, WaitlistStatsResult, SignupRow } from "@/lib/stats";

// The owner's live dashboard. Liveness rides the same public aggregate the
// landing page uses: `db.useQuery("WaitlistStat")` re-renders the instant the
// count changes (cross-tab, via the replica). The emails themselves never sync.
// They come from the owner-gated `waitlistStats` function, fetched on mount and
// again whenever the live count changes, so PII only travels through that call.
export function WaitlistDashboard({ userEmail }: { userEmail: string }) {
  const { data: statRows } = db.useQuery<{ id: string; count: number }>("WaitlistStat");
  const liveCount = statRows.length > 0 ? statRows[0].count : 0;

  const [data, setData] = useState<WaitlistStatsData | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // In `pylon dev` an empty waitlist gets demo signups (functions/seedDemo.ts).
  // On a deploy the call returns without writing.
  useEffect(() => {
    void callFn("seedDemo", {}).catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    callFn<WaitlistStatsResult>("waitlistStats", {})
      .then((r) => {
        if (cancelled) return;
        // The owner gate (PYLON_OWNER_EMAIL) lives in the function; a non-owner
        // comes back as `{ authorized: false }` (with no data) → locked card.
        if (!r.authorized) {
          setDenied(true);
        } else {
          setData(r);
          setError(null);
          setDenied(false);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
    // Re-fetch whenever a new signup moves the live count.
  }, [liveCount]);

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return (
      <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-5 py-4 text-sm text-red-300">
        {error}
      </div>
    );
  }
  if (!data) return <Skeleton />;

  return (
    <div className="space-y-10">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="font-display text-[2rem] font-bold leading-none tracking-[-0.02em] text-chalk">
            Waitlist
          </h1>
          <p className="mt-3 text-[14px] text-chalk-2">
            Signups appear here as people join. The landing page shows the same total.
          </p>
        </div>
        <p className="flex items-center gap-2 font-mono-ui text-[12px] text-chalk-2">
          <span className="relative flex size-2">
            <span className="absolute inline-flex size-2 animate-ping rounded-full bg-brand/60" />
            <span className="relative inline-flex size-2 rounded-full bg-brand" />
          </span>
          live
        </p>
      </div>

      <div className="grid gap-px overflow-hidden rounded-lg border border-line bg-line grid-cols-3">
        <Stat label="Total signups" value={data.total} />
        <Stat label="Last 7 days" value={data.last7} />
        <Stat label="Today" value={data.today} />
      </div>

      <TrendChart daily={data.daily} />

      <SignupTable signups={data.signups} />
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="bg-paper px-3 py-4 sm:px-5 sm:py-5">
      <div className="font-mono-ui text-[12px] text-chalk-2">{label}</div>
      <div className="font-display mt-2 text-[1.75rem] sm:text-[2.25rem] font-bold leading-none tabular-nums tracking-[-0.02em] text-chalk">
        {value.toLocaleString("en-US")}
      </div>
    </div>
  );
}

/* ----------------------------- trend chart ---------------------------- */

function TrendChart({ daily }: { daily: { date: string; count: number }[] }) {
  const max = Math.max(1, ...daily.map((d) => d.count));
  const total = daily.reduce((n, d) => n + d.count, 0);
  return (
    <section className="rounded-lg border border-line bg-paper p-5 sm:p-6">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="font-display text-[17px] font-medium text-chalk">Last 30 days</h2>
        <span className="font-mono-ui text-[12px] text-chalk-2">
          {total.toLocaleString("en-US")} signups · peak {max}/day
        </span>
      </div>
      {/* Each column is a full-height flex cell that bottom-aligns its bar, so
          the bar's percentage height resolves against the track. */}
      <div className="mt-6 flex h-36 items-end gap-[3px]">
        {daily.map((d, i) => (
          <div key={d.date} className="group relative flex h-full flex-1 flex-col justify-end">
            <div
              className={
                "w-full rounded-t-[2px] transition-colors " +
                (i === daily.length - 1 ? "bg-brand" : "bg-brand/45 group-hover:bg-brand/80")
              }
              style={{ height: `${Math.max(2, (d.count / max) * 100)}%` }}
            />
            <div className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-1.5 hidden -translate-x-1/2 whitespace-nowrap rounded border border-line bg-ink px-2 py-1 font-mono-ui text-[11px] text-chalk group-hover:block">
              {fmtDay(d.date)} · {d.count}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-3 flex justify-between border-t border-line pt-2 font-mono-ui text-[11px] text-chalk-2">
        <span>{fmtDay(daily[0]?.date)}</span>
        <span>Today</span>
      </div>
    </section>
  );
}

/* ----------------------------- signup list ---------------------------- */

const PAGE = 50;

function SignupTable({ signups }: { signups: SignupRow[] }) {
  const [q, setQ] = useState("");
  const [shown, setShown] = useState(PAGE);
  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return signups;
    return signups.filter((s) => s.email.toLowerCase().includes(needle));
  }, [q, signups]);
  // Join order: the oldest signup is No. 1.
  const position = useMemo(() => {
    const m = new Map<string, number>();
    signups.forEach((s, i) => m.set(s.id, signups.length - i));
    return m;
  }, [signups]);

  return (
    <section className="overflow-hidden rounded-lg border border-line bg-paper">
      <div className="flex flex-col gap-3 border-b border-line p-4 sm:flex-row sm:items-center sm:justify-between sm:px-5">
        <h2 className="font-display text-[17px] font-medium text-chalk">
          Signups{" "}
          <span className="font-mono-ui text-[13px] font-normal text-chalk-2">
            {filtered.length.toLocaleString("en-US")}
          </span>
        </h2>
        <div className="flex items-center gap-2">
          <label className="relative flex-1 sm:flex-none">
            <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-chalk-2" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search email"
              aria-label="Search signups"
              className="h-9 w-full rounded-md border border-line bg-ink pl-8 pr-3 text-[13px] text-chalk outline-none transition placeholder:text-chalk-2 focus:border-brand sm:w-52"
            />
          </label>
          <button
            type="button"
            onClick={() => exportCsv(signups)}
            disabled={signups.length === 0}
            className="inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md border border-line px-3 text-[13px] font-medium text-chalk transition-colors hover:border-chalk-2 disabled:opacity-40"
          >
            <Download aria-hidden className="size-3.5" />
            Export CSV
          </button>
        </div>
      </div>

      {filtered.length === 0 ? (
        <p className="px-5 py-12 text-center text-[14px] text-chalk-2">
          {signups.length === 0
            ? "No signups yet. They appear here as soon as someone joins."
            : "No emails match your search."}
        </p>
      ) : (
        <>
          <ul className="divide-y divide-line">
            {filtered.slice(0, shown).map((s) => (
              <li key={s.id} className="flex items-center gap-4 px-4 py-3 sm:px-5">
                <span className="hidden w-12 shrink-0 sm:block font-mono-ui text-[12px] tabular-nums text-chalk-2">
                  {position.get(s.id)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[14px] text-chalk">{s.email}</span>
                <span className="shrink-0 font-mono-ui text-[12px] text-chalk-2">
                  {fmtDateTime(s.createdAt)}
                </span>
              </li>
            ))}
          </ul>
          {filtered.length > shown ? (
            <button
              type="button"
              onClick={() => setShown((n) => n + PAGE)}
              className="w-full border-t border-line py-3 text-[13px] text-chalk-2 transition-colors hover:text-chalk"
            >
              Show {Math.min(PAGE, filtered.length - shown)} more
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}

/* --------------------------- owner-only gate -------------------------- */

// Shown to a signed-in user the `waitlistStats` function refused (not the
// configured owner, or no PYLON_OWNER_EMAIL set). Fails closed: no signup data
// is fetched or shown.
function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-lg border border-dashed border-line px-6 py-14 text-center">
      <h1 className="font-display text-[1.5rem] font-bold text-chalk">This dashboard is owner-only</h1>
      <p className="mx-auto mt-3 max-w-md text-[14px] leading-relaxed text-chalk-2">
        You are signed in as <span className="text-chalk">{email || "this account"}</span>. Only
        the owner can see signups. Set{" "}
        <code className="rounded bg-ink-2 px-1.5 py-0.5 font-mono-ui text-[12px] text-chalk">
          PYLON_OWNER_EMAIL={email || "you@yourbusiness.com"}
        </code>{" "}
        in <code className="rounded bg-ink-2 px-1.5 py-0.5 font-mono-ui text-[12px] text-chalk">.env</code>,
        restart, and reload. Or sign in with the owner account.
      </p>
    </div>
  );
}

/* ----------------------------- user menu ------------------------------ */

export function UserMenu({ email }: { email: string }) {
  const { signOut } = useAuth();
  const initial = (email.trim()[0] || "?").toUpperCase();
  async function onSignOut() {
    await signOut();
    window.location.assign("/");
  }
  return (
    <details className="group relative">
      <summary
        aria-label="Account"
        className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-brand text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden"
      >
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-60 overflow-hidden rounded-lg border border-line bg-paper py-1 shadow-[0_24px_48px_-24px_rgba(0,0,0,0.8)]">
        <div className="truncate border-b border-line px-3 py-2.5 text-[13px] text-chalk">
          {email || "Signed in"}
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-chalk-2 transition-colors hover:bg-ink-2 hover:text-chalk"
        >
          <LogOut aria-hidden className="size-3.5" />
          Sign out
        </button>
      </div>
    </details>
  );
}

/* ------------------------------ helpers ------------------------------- */

function Skeleton() {
  return (
    <div className="space-y-10">
      <div className="h-8 w-40 animate-pulse rounded bg-ink-2" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-24 animate-pulse rounded-lg bg-ink-2" />
        ))}
      </div>
      <div className="h-56 animate-pulse rounded-lg bg-ink-2" />
      <div className="h-72 animate-pulse rounded-lg bg-ink-2" />
    </div>
  );
}

function exportCsv(signups: SignupRow[]) {
  const cell = (v: string) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
  const rows = ["email,joined_at", ...signups.map((s) => `${cell(s.email)},${cell(s.createdAt)}`)];
  const blob = new Blob([rows.join("\n")], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `waitlist-${new Date().toISOString().slice(0, 10)}.csv`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function fmtDay(iso?: string) {
  if (!iso) return "";
  const d = new Date(iso + "T00:00:00Z");
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

function fmtDateTime(iso: string) {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  return new Date(t).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
