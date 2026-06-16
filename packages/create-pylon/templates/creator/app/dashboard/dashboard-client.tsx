"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import type { SubscriberStatsData, SubscriberStatsResult, SubscriberRow } from "@/lib/stats";

// The owner's live dashboard. Liveness rides the SAME public aggregate the
// landing page uses: `db.useQuery("SubscriberCount")` re-renders the instant the
// count changes (cross-tab, via the replica). The emails themselves never sync
// — they come from the owner-gated `subscriberStats` function, (re)fetched on
// mount and whenever the live count ticks. So the total, chart, and list all
// stay live, but PII only ever travels through the gated call.
export function SubscriberDashboard({ userEmail }: { userEmail: string }) {
  const { data: statRows } = db.useQuery<{ id: string; count: number }>("SubscriberCount");
  const liveCount = statRows.length > 0 ? statRows[0].count : 0;

  const [data, setData] = useState<SubscriberStatsData | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
      <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">
        {error}
      </div>
    );
  }
  if (!data) return <Skeleton />;

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Subscribers</h1>
        <p className="mt-1 text-sm text-zinc-500">
          Live — new subscribers appear here the moment they sign up.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Total subscribers" value={data.total} />
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
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">
        {label}
      </div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">
        {value.toLocaleString()}
      </div>
    </div>
  );
}

/* ----------------------------- trend chart ---------------------------- */

function TrendChart({ daily }: { daily: { date: string; count: number }[] }) {
  const max = Math.max(1, ...daily.map((d) => d.count));
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-5">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-900">Last 30 days</h2>
        <span className="text-[12px] text-zinc-400">
          peak {max.toLocaleString()}/day
        </span>
      </div>
      {/* Each column is a FULL-HEIGHT flex cell (h-full) that bottom-aligns its
          bar — so the bar's percentage height resolves against the 7rem track,
          not an auto-height parent (which would collapse it to nothing). */}
      <div className="mt-4 flex h-28 items-end gap-1">
        {daily.map((d) => (
          <div
            key={d.date}
            className="group relative flex h-full flex-1 flex-col justify-end"
          >
            <div
              className="w-full rounded-t bg-brand/80 transition-colors group-hover:bg-brand"
              style={{ height: `${Math.max(2, (d.count / max) * 100)}%` }}
            />
            {/* Tooltip on hover — date + count. */}
            <div className="pointer-events-none absolute bottom-full left-1/2 z-10 mb-1 hidden -translate-x-1/2 whitespace-nowrap rounded bg-zinc-900 px-2 py-1 text-[11px] text-white group-hover:block">
              {fmtDay(d.date)} · {d.count}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-2 flex justify-between text-[11px] text-zinc-400">
        <span>{fmtDay(daily[0]?.date)}</span>
        <span>{fmtDay(daily[daily.length - 1]?.date)}</span>
      </div>
    </div>
  );
}

/* ----------------------------- subscriber list ---------------------------- */

function SubscriberTable({ subscribers }: { subscribers: SubscriberRow[] }) {
  const [q, setQ] = useState("");
  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return subscribers;
    return subscribers.filter((s) => s.email.toLowerCase().includes(needle));
  }, [q, subscribers]);

  return (
    <div className="rounded-xl border border-zinc-200 bg-white">
      <div className="flex flex-col gap-3 border-b border-zinc-100 p-4 sm:flex-row sm:items-center sm:justify-between">
        <h2 className="text-sm font-semibold text-zinc-900">
          Subscribers{" "}
          <span className="font-normal text-zinc-400">
            ({filtered.length.toLocaleString()})
          </span>
        </h2>
        <div className="flex items-center gap-2">
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search email…"
            aria-label="Search subscribers"
            className="h-9 w-44 rounded-md border border-zinc-300 bg-white px-3 text-sm outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20"
          />
          <button
            type="button"
            onClick={() => exportCsv(subscribers)}
            disabled={subscribers.length === 0}
            className="inline-flex h-9 items-center rounded-md border border-zinc-300 px-3 text-[13px] font-medium text-zinc-700 transition-colors hover:bg-zinc-50 disabled:opacity-40"
          >
            Export CSV
          </button>
        </div>
      </div>

      {filtered.length === 0 ? (
        <p className="p-8 text-center text-sm text-zinc-500">
          {subscribers.length === 0
            ? "No subscribers yet. Share your landing page and watch them roll in — live."
            : "No emails match your search."}
        </p>
      ) : (
        <ul className="divide-y divide-zinc-100">
          {filtered.map((s) => (
            <li key={s.id} className="flex items-center justify-between gap-3 px-4 py-2.5">
              <span className="truncate text-sm text-zinc-800">{s.email}</span>
              <span className="shrink-0 text-[12px] text-zinc-400">
                {fmtDateTime(s.createdAt)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/* --------------------------- owner-only gate -------------------------- */

// Shown to a signed-in user the `subscriberStats` function refused (not the
// configured owner, or no PYLON_OWNER_EMAIL set). Fails closed — no subscriber data
// is fetched or shown.
function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as{" "}
        <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        A newsletter is single-tenant — only the owner can see the subscribers. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">
          PYLON_OWNER_EMAIL={email || "you@yourbusiness.com"}
        </code>{" "}
        in your{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">.env</code>,
        restart, and reload — or sign in with the owner account.
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
      <summary className="flex size-8 cursor-pointer select-none list-none items-center justify-center rounded-full bg-zinc-900 text-[12px] font-semibold text-white marker:hidden [&::-webkit-details-marker]:hidden">
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-56 overflow-hidden rounded-xl border border-zinc-200 bg-white py-1 shadow-[0_16px_48px_-16px_rgba(0,0,0,0.25)]">
        <div className="border-b border-zinc-100 px-3 py-2">
          <div className="truncate text-[13px] font-medium text-zinc-900">
            {email || "Signed in"}
          </div>
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center px-3 py-2 text-left text-[13px] text-zinc-700 transition-colors hover:bg-zinc-50"
        >
          Sign out
        </button>
      </div>
    </details>
  );
}

/* ------------------------------ helpers ------------------------------- */

function Skeleton() {
  return (
    <div className="space-y-8">
      <div className="h-6 w-32 animate-pulse rounded bg-zinc-100" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-44 animate-pulse rounded-xl bg-zinc-100" />
      <div className="h-64 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}

function exportCsv(subscribers: SubscriberRow[]) {
  const cell = (v: string) =>
    /[",\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
  const rows = [
    "email,joined_at",
    ...subscribers.map((s) => `${cell(s.email)},${cell(s.createdAt)}`),
  ];
  const blob = new Blob([rows.join("\n")], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `newsletter-${new Date().toISOString().slice(0, 10)}.csv`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

function fmtDay(iso?: string) {
  if (!iso) return "";
  const d = new Date(iso + "T00:00:00Z");
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function fmtDateTime(iso: string) {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  return new Date(t).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}
