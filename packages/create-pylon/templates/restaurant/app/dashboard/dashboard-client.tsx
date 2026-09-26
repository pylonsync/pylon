"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Check, LogOut, MapPin, X } from "lucide-react";
import { siteConfig } from "@/lib/site.config";
import { localDateKey } from "@/lib/slots";
import type { ReservationRow, OwnerReservationsResult } from "@/lib/reservation";

// The owner's live reservations dashboard. Liveness rides the SAME public
// ReservationSlot markers the picker uses: `db.useQuery` re-renders the instant
// a table is taken or freed, triggering a re-fetch of the owner-gated
// `reservationsForOwner` — so new reservations appear and cancellations drop off
// without a refresh. Guest PII only travels through that gated call.
export function ReservationsDashboard({ userEmail }: { userEmail: string }) {
  const { data: markers } = db.useQuery<{ id: string }>("ReservationSlot");
  const liveKey = markers.length;

  const [reservations, setReservations] = useState<ReservationRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [day, setDay] = useState<string | null>(null);

  async function load() {
    try {
      const r = await callFn<OwnerReservationsResult>("reservationsForOwner", {});
      if (!r.authorized) setDenied(true);
      else {
        setReservations(r.reservations);
        setDenied(false);
        setError(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // In `pylon dev` an empty book gets demo reservations (functions/seedDemo.ts).
  // On a deploy the call returns without writing.
  useEffect(() => {
    void callFn("seedDemo", {}).catch(() => {});
  }, []);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);

  async function act(id: string, fn: "confirmReservation" | "cancelReservation") {
    setBusyId(id);
    try {
      await callFn(fn, { reservationId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  const nowMs = Date.now();
  // The next 14 open days, each with its reservations.
  const days = useMemo(() => {
    const cfg = siteConfig.reservations;
    const byDay = new Map<string, ReservationRow[]>();
    for (const r of reservations ?? []) {
      const key = dayKey(r.startsAt);
      const list = byDay.get(key) ?? [];
      list.push(r);
      byDay.set(key, list);
    }
    const out: { key: string; rows: ReservationRow[] }[] = [];
    for (let i = 0; i < 21 && out.length < 14; i++) {
      const key = localDateKey(i, nowMs);
      const [y, m, d] = key.split("-").map(Number);
      if (!cfg.hours[new Date(y, m - 1, d).getDay()]) continue;
      const rows = (byDay.get(key) ?? []).sort((a, b) => (a.startsAt < b.startsAt ? -1 : 1));
      out.push({ key, rows });
    }
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reservations]);

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return <div className="rounded-lg border border-brand/40 bg-brand/10 px-5 py-4 text-sm text-[var(--cream)]">{error}</div>;
  }
  if (!reservations) return <Skeleton />;

  const active = reservations.filter((r) => r.status !== "cancelled");
  const upcoming = active.filter((r) => Date.parse(r.startsAt) >= nowMs - 3_600_000);
  const todayKey = localDateKey(0, nowMs);
  // Every active table today, seated or not, so the stat matches the day chip.
  const tonight = active.filter((r) => dayKey(r.startsAt) === todayKey);
  const weekEnd = nowMs + 7 * 86_400_000;
  const weekCovers = upcoming
    .filter((r) => Date.parse(r.startsAt) < weekEnd)
    .reduce((sum, r) => sum + (r.partySize || 0), 0);
  const pendingCount = upcoming.filter((r) => r.status === "pending").length;

  const selected = days.find((d) => d.key === day) ?? days[0];

  return (
    <div className="space-y-10">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="font-display text-[2.75rem] font-medium leading-none tracking-[-0.01em] sm:text-[3.5rem]">
            Reservations
          </h1>
          <p className="mt-3 max-w-lg text-[15px] leading-relaxed text-[var(--cream-2)]">
            New reservations appear here as guests book. Cancelling one frees the table on the
            site right away.
          </p>
        </div>
        <p className="flex items-center gap-2 text-[13px] text-[var(--cream-2)]">
          <span className="relative flex size-2">
            <span className="absolute inline-flex size-2 animate-ping rounded-full bg-brand/60" />
            <span className="relative inline-flex size-2 rounded-full bg-brand" />
          </span>
          Live
        </p>
      </div>

      <dl className="grid grid-cols-2 gap-px overflow-hidden rounded-[6px] border border-white/10 bg-white/10 sm:grid-cols-4">
        <Stat label="Tonight" value={`${tonight.length}`} note={`${tonight.reduce((n, r) => n + r.partySize, 0)} covers`} />
        <Stat label="Covers, next 7 days" value={weekCovers.toLocaleString("en-US")} />
        <Stat label="Upcoming" value={upcoming.length.toLocaleString("en-US")} />
        <Stat label="To confirm" value={`${pendingCount}`} accent={pendingCount > 0} />
      </dl>

      <section>
        <div className="flex gap-2 overflow-x-auto pb-2">
          {days.map((d) => {
            const isOn = d.key === selected?.key;
            const covers = d.rows.filter((r) => r.status !== "cancelled").reduce((n, r) => n + r.partySize, 0);
            return (
              <button
                key={d.key}
                type="button"
                onClick={() => setDay(d.key)}
                aria-pressed={isOn}
                className={
                  "flex shrink-0 flex-col items-start whitespace-nowrap rounded-[6px] border px-3.5 py-2.5 text-left transition-colors " +
                  (isOn
                    ? "border-brand bg-brand text-white"
                    : "border-white/10 bg-[var(--ink-2)] text-[var(--cream)] hover:border-white/30")
                }
              >
                <span className={"text-[12px] " + (isOn ? "text-white/80" : "text-[var(--cream-2)]")}>
                  {d.key === todayKey ? "Tonight" : weekday(d.key)}
                </span>
                <span className="font-display text-[20px] leading-tight">{monthDay(d.key)}</span>
                <span className={"mt-1 text-[12px] tabular-nums " + (isOn ? "text-white/80" : "text-[var(--cream-2)]")}>
                  {covers} covers
                </span>
              </button>
            );
          })}
        </div>

        {selected ? (
          <ServiceSheet rows={selected.rows} busyId={busyId} onAct={act} />
        ) : null}
      </section>

      {!siteConfig.location.mapEmbedUrl ? <MapHint /> : null}
    </div>
  );
}

function dayKey(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}
function keyDate(key: string): Date {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d);
}
function weekday(key: string) {
  return keyDate(key).toLocaleDateString("en-US", { weekday: "short" });
}
function monthDay(key: string) {
  return keyDate(key).toLocaleDateString("en-US", { month: "short", day: "numeric" });
}
function timeOf(iso: string) {
  return new Date(iso).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
}

function Stat({ label, value, note, accent }: { label: string; value: string; note?: string; accent?: boolean }) {
  return (
    <div className="bg-[var(--ink)] px-4 py-4 sm:px-5">
      <dt className="text-[12px] text-[var(--cream-2)]">{label}</dt>
      <dd className="mt-1.5 flex items-baseline gap-2">
        <span className={"font-display text-[2.25rem] leading-none tabular-nums " + (accent ? "text-brand" : "")}>
          {value}
        </span>
        {note ? <span className="text-[12px] text-[var(--cream-2)]">{note}</span> : null}
      </dd>
    </div>
  );
}

// One day's book, grouped by seating time.
function ServiceSheet({
  rows,
  busyId,
  onAct,
}: {
  rows: ReservationRow[];
  busyId: string | null;
  onAct: (id: string, fn: "confirmReservation" | "cancelReservation") => void;
}) {
  const seatings = useMemo(() => {
    const m = new Map<string, ReservationRow[]>();
    for (const r of rows) {
      const list = m.get(r.startsAt) ?? [];
      list.push(r);
      m.set(r.startsAt, list);
    }
    return Array.from(m.entries());
  }, [rows]);

  if (rows.length === 0) {
    return (
      <div className="mt-4 rounded-[6px] border border-dashed border-white/15 p-10 text-center text-[15px] text-[var(--cream-2)]">
        No reservations for this day yet.
      </div>
    );
  }

  return (
    <div className="mt-4 overflow-hidden rounded-[6px] border border-white/10">
      {seatings.map(([startsAt, list]) => (
        <div key={startsAt} className="grid border-b border-white/10 last:border-b-0 sm:grid-cols-[7rem_1fr]">
          <div className="border-white/10 bg-[var(--ink-2)] px-4 py-3 sm:border-r">
            <div className="font-display text-[20px] leading-none">{timeOf(startsAt)}</div>
            <div className="mt-1 text-[12px] text-[var(--cream-2)]">
              {list.filter((r) => r.status !== "cancelled").length}/{siteConfig.reservations.tablesPerSlot} tables
            </div>
          </div>
          <ul className="divide-y divide-white/10">
            {list.map((r) => (
              <Row key={r.id} r={r} busy={busyId === r.id} onAct={onAct} />
            ))}
          </ul>
        </div>
      ))}
    </div>
  );
}

function Row({
  r,
  busy,
  onAct,
}: {
  r: ReservationRow;
  busy: boolean;
  onAct: (id: string, fn: "confirmReservation" | "cancelReservation") => void;
}) {
  const cancelled = r.status === "cancelled";
  const past = Date.parse(r.startsAt) < Date.now() - 3_600_000;
  return (
    <li className={"flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 " + (cancelled ? "opacity-50" : "")}>
      <span
        aria-label={`Party of ${r.partySize}`}
        className="flex size-9 shrink-0 items-center justify-center rounded-full border border-white/15 font-display text-[17px] tabular-nums"
      >
        {r.partySize}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={"text-[15px] font-medium " + (cancelled ? "line-through" : "")}>{r.customerName}</span>
          <StatusTag status={r.status} />
        </div>
        <div className="mt-0.5 truncate text-[13px] text-[var(--cream-2)]">
          {r.notes ? <span className="italic text-[var(--cream)]">{r.notes} · </span> : null}
          {r.customerPhone ?? r.customerEmail}
        </div>
      </div>
      {!cancelled && !past ? (
        <div className="flex items-center gap-2">
          {r.status === "pending" ? (
            <button
              type="button"
              disabled={busy}
              onClick={() => onAct(r.id, "confirmReservation")}
              className="inline-flex h-8 items-center gap-1.5 rounded-full bg-brand px-3.5 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
            >
              <Check aria-hidden className="size-3.5" />
              Confirm
            </button>
          ) : null}
          <button
            type="button"
            disabled={busy}
            onClick={() => onAct(r.id, "cancelReservation")}
            className="inline-flex h-8 items-center gap-1.5 rounded-full border border-white/20 px-3.5 text-[13px] font-medium text-[var(--cream-2)] transition-colors hover:border-white/40 hover:text-[var(--cream)] disabled:opacity-50"
          >
            <X aria-hidden className="size-3.5" />
            Cancel
          </button>
        </div>
      ) : null}
    </li>
  );
}

function StatusTag({ status }: { status: string }) {
  if (status === "confirmed") return null;
  const label = status === "pending" ? "To confirm" : "Cancelled";
  const tone = status === "pending" ? "bg-brand/20 text-[#f3b58c]" : "bg-white/10 text-[var(--cream-2)]";
  return <span className={"rounded-full px-2 py-0.5 text-[11px] font-medium " + tone}>{label}</span>;
}

// Setup notes for the owner live here, not on the public site.
function MapHint() {
  return (
    <aside className="flex gap-3 rounded-[6px] border border-white/10 bg-[var(--ink-2)] p-4 text-[13.5px] leading-relaxed text-[var(--cream-2)]">
      <MapPin aria-hidden className="mt-0.5 size-4 shrink-0 text-[var(--cream)]" />
      <p>
        Your site shows the week&apos;s hours next to the address. To show a map there instead,
        paste a Google Maps embed URL into{" "}
        <code className="rounded bg-black/30 px-1 text-[12.5px] text-[var(--cream)]">location.mapEmbedUrl</code> in{" "}
        <code className="rounded bg-black/30 px-1 text-[12.5px] text-[var(--cream)]">lib/site.config.ts</code>.
      </p>
    </aside>
  );
}

/* --------------------------- owner-only gate -------------------------- */

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-[6px] border border-dashed border-white/15 px-6 py-12 text-center">
      <h1 className="font-display text-[2rem] leading-tight">This dashboard is owner-only</h1>
      <p className="mx-auto mt-3 max-w-md text-[14px] leading-relaxed text-[var(--cream-2)]">
        You are signed in as <span className="text-[var(--cream)]">{email || "this account"}</span>.
        Only the restaurant owner can see reservations. Set{" "}
        <code className="rounded bg-black/30 px-1.5 py-0.5 text-[12px] text-[var(--cream)]">
          PYLON_OWNER_EMAIL={email || "you@yourrestaurant.com"}
        </code>{" "}
        in <code className="rounded bg-black/30 px-1.5 py-0.5 text-[12px] text-[var(--cream)]">.env</code>,
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
      <div className="absolute right-0 top-full z-40 mt-2 w-60 overflow-hidden rounded-[6px] border border-white/10 bg-[var(--ink-2)] py-1 shadow-[0_24px_48px_-24px_rgba(0,0,0,0.8)]">
        <div className="truncate border-b border-white/10 px-3 py-2.5 text-[13px] text-[var(--cream)]">
          {email || "Signed in"}
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-[var(--cream-2)] transition-colors hover:bg-white/5 hover:text-[var(--cream)]"
        >
          <LogOut aria-hidden className="size-3.5" />
          Sign out
        </button>
      </div>
    </details>
  );
}

/* ------------------------------ skeleton ------------------------------ */

function Skeleton() {
  return (
    <div className="space-y-10">
      <div className="h-14 w-72 animate-pulse rounded bg-white/5" />
      <div className="h-24 animate-pulse rounded-[6px] bg-white/5" />
      <div className="h-20 animate-pulse rounded-[6px] bg-white/5" />
      <div className="h-72 animate-pulse rounded-[6px] bg-white/5" />
    </div>
  );
}
