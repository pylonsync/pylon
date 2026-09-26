"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Download, LogOut, Search } from "lucide-react";
import type { SubscriberStatsData, SubscriberStatsResult, SubscriberRow } from "@/lib/stats";

// The owner's live dashboard. Liveness rides the same public aggregate the
// landing page uses: `db.useQuery("SubscriberCount")` re-renders the instant the
// count changes (cross-tab, via the replica). The emails themselves never sync.
// They come from the owner-gated `subscriberStats` function, fetched on mount and
// again whenever the live count changes, so PII only travels through that call.
export function SubscriberDashboard({ userEmail }: { userEmail: string }) {
  const { data: statRows } = db.useQuery<{ id: string; count: number }>("SubscriberCount");
  const liveCount = statRows.length > 0 ? statRows[0].count : 0;

  const [data, setData] = useState<SubscriberStatsData | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // In `pylon dev` an empty list gets demo subscribers (functions/seedDemo.ts).
  // On a deploy the call returns without writing.
  useEffect(() => {
    void callFn("seedDemo", {}).catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    callFn<SubscriberStatsResult>("subscriberStats", {})
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
    // Re-fetch whenever a new subscriber moves the live count.
  }, [liveCount]);

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">
        {error}
      </div>
    );
  }
  if (!data) return <Skeleton />;

  return (
    <div className="space-y-10">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-[2.25rem] font-medium leading-none text-ink">
            Subscribers
          </h1>
          <p className="mt-3 text-[16px] text-ink-2">
            New subscribers appear here as they sign up. Your site shows the same total.
          </p>
        </div>
        <p className="flex items-center gap-2 text-[13px] text-ink-2">
          <span className="relative flex size-2">
            <span className="absolute inline-flex size-2 animate-ping rounded-full bg-brand/60" />
            <span className="relative inline-flex size-2 rounded-full bg-brand" />
          </span>
          live
        </p>
      </div>

      <div className="grid gap-px overflow-hidden rounded-lg border border-rule bg-rule grid-cols-3">
        <Stat label="Subscribers" value={data.total} />
        <Stat label="Last 7 days" value={data.last7} />
        <Stat label="Today" value={data.today} />
      </div>

      <TrendChart daily={data.daily} />

      <SubscriberTable subscribers={data.subscribers} />
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="bg-white px-3 py-4 sm:px-5 sm:py-5">
      <div className="text-[13px] text-ink-2">{label}</div>
      <div className="mt-2 text-[1.75rem] sm:text-[2.25rem] font-medium leading-none tabular-nums text-ink">
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
    <section className="rounded-lg border border-rule bg-white p-5 sm:p-6">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-[19px] font-medium text-ink">Last 30 days</h2>
        <span className="text-[13px] text-ink-2">
          {total.toLocaleString("en-US")} new · peak {max}/day
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
            <div className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-1.5 hidden -translate-x-1/2 whitespace-nowrap rounded border border-rule bg-ink px-2 py-1 text-[12px] text-paper group-hover:block">
              {fmtDay(d.date)} · {d.count}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-3 flex justify-between border-t border-rule pt-2 text-[11px] text-ink-2">
        <span>{fmtDay(daily[0]?.date)}</span>
        <span>Today</span>
      </div>
    </section>
  );
}

/* --------------------------- subscriber list -------------------------- */

const PAGE = 50;

function SubscriberTable({ subscribers }: { subscribers: SubscriberRow[] }) {
  const [q, setQ] = useState("");
  const [shown, setShown] = useState(PAGE);
  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return subscribers;
    return subscribers.filter((s) => s.email.toLowerCase().includes(needle));
  }, [q, subscribers]);
  // Join order: the oldest subscriber is No. 1.
  const position = useMemo(() => {
    const m = new Map<string, number>();
    subscribers.forEach((s, i) => m.set(s.id, subscribers.length - i));
    return m;
  }, [subscribers]);

  return (
    <section className="overflow-hidden rounded-lg border border-rule bg-white">
      <div className="flex flex-col gap-3 border-b border-rule p-4 sm:flex-row sm:items-center sm:justify-between sm:px-5">
        <h2 className="text-[19px] font-medium text-ink">
          All subscribers{" "}
          <span className="text-[13px] font-normal text-ink-2">
            {filtered.length.toLocaleString("en-US")}
          </span>
        </h2>
        <div className="flex items-center gap-2">
          <label className="relative flex-1 sm:flex-none">
            <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-2" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder="Search email"
              aria-label="Search subscribers"
              className="h-9 w-full rounded-md border border-rule bg-white pl-8 pr-3 text-[13px] text-ink outline-none transition placeholder:text-ink-2/60 focus:border-brand sm:w-52"
            />
          </label>
          <button
            type="button"
            onClick={() => exportCsv(subscribers)}
            disabled={subscribers.length === 0}
            className="inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md border border-rule px-3 text-[13px] font-medium text-ink transition-colors hover:border-ink-2 disabled:opacity-40"
          >
            <Download aria-hidden className="size-3.5" />
            Export CSV
          </button>
        </div>
      </div>

      {filtered.length === 0 ? (
        <p className="px-5 py-12 text-center text-[16px] text-ink-2">
          {subscribers.length === 0
            ? "No subscribers yet. They appear here as soon as someone signs up."
            : "No emails match your search."}
        </p>
      ) : (
        <>
          <ul className="divide-y divide-rule">
            {filtered.slice(0, shown).map((s) => (
              <li key={s.id} className="flex items-center gap-4 px-4 py-3 sm:px-5">
                <span className="hidden w-12 shrink-0 sm:block text-[12px] tabular-nums text-ink-2">
                  {position.get(s.id)}
                </span>
                <span className="min-w-0 flex-1 truncate text-[14px] text-ink">{s.email}</span>
                <span className="shrink-0 text-[13px] text-ink-2">
                  {fmtDateTime(s.createdAt)}
                </span>
              </li>
            ))}
          </ul>
          {filtered.length > shown ? (
            <button
              type="button"
              onClick={() => setShown((n) => n + PAGE)}
              className="w-full border-t border-rule py-3 text-[13px] text-ink-2 transition-colors hover:text-ink"
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

// Shown to a signed-in user the `subscriberStats` function refused (not the
// configured owner, or no PYLON_OWNER_EMAIL set). Fails closed: no subscriber data
// is fetched or shown.
function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-lg border border-dashed border-rule px-6 py-14 text-center">
      <h1 className="text-[1.5rem] font-medium text-ink">This dashboard is owner-only</h1>
      <p className="mx-auto mt-3 max-w-md text-[14px] leading-relaxed text-ink-2">
        You are signed in as <span className="text-ink">{email || "this account"}</span>. Only
        the owner can see subscribers. Set{" "}
        <code className="rounded bg-rule/50 px-1.5 py-0.5 text-[12px] text-ink">
          PYLON_OWNER_EMAIL={email || "you@yoursite.com"}
        </code>{" "}
        in <code className="rounded bg-rule/50 px-1.5 py-0.5 text-[12px] text-ink">.env</code>,
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
      <div className="absolute right-0 top-full z-40 mt-2 w-60 overflow-hidden rounded-lg border border-rule bg-white py-1 shadow-[0_24px_48px_-24px_rgba(42,38,34,0.35)]">
        <div className="truncate border-b border-rule px-3 py-2.5 text-[13px] text-ink">
          {email || "Signed in"}
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-ink-2 transition-colors hover:bg-rule/50 hover:text-ink"
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
      <div className="h-8 w-40 animate-pulse rounded bg-rule/50" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-24 animate-pulse rounded-lg bg-rule/50" />
        ))}
      </div>
      <div className="h-56 animate-pulse rounded-lg bg-rule/50" />
      <div className="h-72 animate-pulse rounded-lg bg-rule/50" />
    </div>
  );
}

function exportCsv(subscribers: SubscriberRow[]) {
  const cell = (v: string) => (/[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
  const rows = ["email,subscribed_at", ...subscribers.map((s) => `${cell(s.email)},${cell(s.createdAt)}`)];
  const blob = new Blob([rows.join("\n")], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `subscribers-${new Date().toISOString().slice(0, 10)}.csv`;
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
