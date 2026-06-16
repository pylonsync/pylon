"use client";

import React, { useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import { seatingsForDay, weekdayOf, localDateKey } from "@/lib/slots";

// The live reservation picker — the realtime heart of this template. It
// subscribes to the public, PII-free `ReservationSlot` markers with
// `db.useQuery` and COUNTS them per seating, so "4 left" ticks down — and a
// time greys out to "Full" for everyone with the page open — the instant the
// last table goes. The server re-checks the count at insert (under a per-seating
// lock), so two parties can't both grab the last table.
//
// Wrapped in <EnsureGuest> so the sync connection is established for anonymous
// visitors. The guest session holds no PII and can't read the Reservation
// table; the picker only ever reads bare seating timestamps.

interface SlotMarker {
  id: string;
  startsAt: string;
}

export function ReservationWidget() {
  return (
    <EnsureGuest fallback={<PickerSkeleton />}>
      <Picker />
    </EnsureGuest>
  );
}

function Picker() {
  const cfg = siteConfig.reservations;
  const { data: markers, loading } = db.useQuery<SlotMarker>("ReservationSlot");
  const [nowMs] = useState(() => Date.now());

  const openDays = useMemo(() => {
    const days: string[] = [];
    for (let i = 0; i < cfg.daysAhead; i++) {
      const key = localDateKey(i, nowMs);
      if (cfg.hours[weekdayOf(key)]) days.push(key);
    }
    return days;
  }, [cfg.daysAhead, cfg.hours, nowMs]);

  const [day, setDay] = useState(openDays[0] ?? "");
  const [party, setParty] = useState(2);
  const [selected, setSelected] = useState<string | null>(null); // seating ISO
  const [confirmed, setConfirmed] = useState<{ startsAt: string; party: number } | null>(null);

  // Count markers per seating → remaining tables. db.useQuery is live, so this
  // recomputes the instant anyone reserves or cancels (here or in another tab).
  const takenByTime = useMemo(() => {
    const m = new Map<string, number>();
    for (const row of markers) m.set(row.startsAt, (m.get(row.startsAt) ?? 0) + 1);
    return m;
  }, [markers]);

  const seatings = useMemo(() => {
    if (!day) return [];
    const hrs = cfg.hours[weekdayOf(day)];
    if (!hrs) return [];
    return seatingsForDay({
      dayISODate: day,
      open: hrs.open,
      close: hrs.close,
      slotMinutes: cfg.slotMinutes,
      leadTimeHours: cfg.leadTimeHours,
      nowMs,
    }).map((s) => {
      const remaining = cfg.tablesPerSlot - (takenByTime.get(s.startsAt) ?? 0);
      return { startsAt: s.startsAt, remaining, available: !s.past && remaining > 0 };
    });
  }, [day, cfg, takenByTime, nowMs]);

  // Clear an in-progress selection if its seating just filled up (live).
  const selectedSeat = selected ? seatings.find((s) => s.startsAt === selected) : null;
  if (selected && (!selectedSeat || !selectedSeat.available)) {
    queueMicrotask(() => setSelected(null));
  }

  function pickSeat(startsAt: string) {
    setConfirmed(null);
    setSelected(startsAt);
  }

  return (
    <div className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm sm:p-7">
      {/* Date picker */}
      <div className="flex gap-2 overflow-x-auto pb-1">
        {openDays.map((key) => {
          const active = key === day;
          return (
            <button
              key={key}
              type="button"
              onClick={() => {
                setDay(key);
                setSelected(null);
                setConfirmed(null);
              }}
              className={
                "flex shrink-0 flex-col items-center rounded-xl border px-3 py-2 transition-colors " +
                (active ? "border-brand bg-brand-soft" : "border-zinc-200 hover:border-zinc-300")
              }
            >
              <span className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">
                {dowOfKey(key)}
              </span>
              <span
                className={"text-[15px] font-semibold " + (active ? "text-brand" : "text-zinc-900")}
              >
                {dayOfKey(key)}
              </span>
            </button>
          );
        })}
      </div>

      {/* Party size */}
      <div className="mt-5 flex flex-wrap items-center gap-2">
        <span className="text-[13px] font-medium text-zinc-500">Party</span>
        {Array.from({ length: cfg.maxPartySize }, (_, i) => i + 1).map((n) => (
          <button
            key={n}
            type="button"
            onClick={() => setParty(n)}
            className={
              "size-8 rounded-full border text-[13px] font-medium transition-colors " +
              (n === party
                ? "border-zinc-900 bg-zinc-900 text-white"
                : "border-zinc-300 text-zinc-700 hover:border-zinc-400")
            }
          >
            {n}
          </button>
        ))}
      </div>

      {/* Seatings */}
      <div className="mt-5 border-t border-zinc-100 pt-5">
        {loading ? (
          <SeatingsSkeleton />
        ) : seatings.length === 0 ? (
          <p className="py-6 text-center text-sm text-zinc-500">Closed that day — pick another.</p>
        ) : (
          <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
            {seatings.map((seat) => {
              const isSelected = selected === seat.startsAt;
              return (
                <button
                  key={seat.startsAt}
                  type="button"
                  disabled={!seat.available}
                  onClick={() => pickSeat(seat.startsAt)}
                  aria-pressed={isSelected}
                  className={
                    "flex flex-col items-center rounded-lg border py-2 text-[13px] font-medium tabular-nums transition-colors " +
                    (!seat.available
                      ? "cursor-not-allowed border-zinc-100 bg-zinc-50 text-zinc-300"
                      : isSelected
                        ? "border-brand bg-brand text-white"
                        : "border-zinc-200 text-zinc-800 hover:border-brand hover:text-brand")
                  }
                >
                  <span>{labelTime(seat.startsAt)}</span>
                  <span
                    className={
                      "text-[10px] font-normal " +
                      (!seat.available
                        ? "text-zinc-300"
                        : isSelected
                          ? "text-white/70"
                          : "text-zinc-400")
                    }
                  >
                    {seat.available ? `${seat.remaining} left` : "Full"}
                  </span>
                </button>
              );
            })}
          </div>
        )}
        <p className="mt-3 text-[12px] text-zinc-400">
          Tables left updates live as others reserve.
        </p>
      </div>

      {confirmed ? (
        <div className="mt-6 rounded-xl border border-brand/30 bg-brand-soft/60 p-5 text-center">
          <p className="text-[15px] font-semibold text-zinc-900">
            Table for {confirmed.party} · {dowOfKey(confirmed.startsAt.slice(0, 10))}{" "}
            {labelDay(confirmed.startsAt)} at {labelTime(confirmed.startsAt)}
          </p>
          <p className="mt-2 text-[14px] text-zinc-600">{siteConfig.reservations.confirmationMessage}</p>
        </div>
      ) : selected ? (
        <ReservationForm
          startsAt={selected}
          party={party}
          onClear={() => setSelected(null)}
          onBooked={() => {
            setConfirmed({ startsAt: selected, party });
            setSelected(null);
          }}
        />
      ) : null}
    </div>
  );
}

function ReservationForm({
  startsAt,
  party,
  onClear,
  onBooked,
}: {
  startsAt: string;
  party: number;
  onClear: () => void;
  onBooked: () => void;
}) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [phone, setPhone] = useState("");
  const [notes, setNotes] = useState("");
  const [status, setStatus] = useState<"idle" | "booking">("idle");
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (status === "booking") return;
    if (!name.trim() || !email.trim()) {
      setError("Name and email are required.");
      return;
    }
    setStatus("booking");
    setError(null);
    try {
      const res = await callFn<{ ok: boolean; reason?: string }>("createReservation", {
        startsAt,
        partySize: party,
        customerName: name.trim(),
        customerEmail: email.trim(),
        customerPhone: phone.trim() || undefined,
        notes: notes.trim() || undefined,
      });
      if (res.ok) {
        onBooked();
      } else {
        setStatus("idle");
        setError(
          res.reason === "full"
            ? "That seating just filled up — pick another time."
            : res.reason === "past"
              ? "That's too soon — choose a later time."
              : res.reason === "party"
                ? "Please call us for a party that size."
                : "Couldn't reserve that seating. Try another.",
        );
        if (res.reason === "full") onClear();
      }
    } catch (err) {
      setStatus("idle");
      setError(
        /valid email|name/i.test(err instanceof Error ? err.message : "")
          ? "Check your name and email."
          : "Something went wrong — try again.",
      );
    }
  }

  return (
    <form onSubmit={submit} className="mt-6 rounded-xl border border-zinc-200 bg-paper p-5">
      <div className="flex items-center justify-between">
        <div className="text-[14px] font-medium text-zinc-900">
          Table for {party} · {labelDow(startsAt)} {labelDay(startsAt)} at {labelTime(startsAt)}
        </div>
        <button
          type="button"
          onClick={onClear}
          className="text-[13px] text-zinc-400 transition-colors hover:text-zinc-700"
        >
          Change
        </button>
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Your name" aria-label="Your name" autoComplete="name" className={inputCls} />
        <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@email.com" aria-label="Email" autoComplete="email" className={inputCls} />
        <input type="tel" value={phone} onChange={(e) => setPhone(e.target.value)} placeholder="Phone (optional)" aria-label="Phone" autoComplete="tel" className={inputCls} />
        <input value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="Notes (allergies, occasion…)" aria-label="Notes" className={inputCls} />
      </div>
      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "booking"}
        className="mt-4 inline-flex h-10 w-full items-center justify-center rounded-lg bg-brand text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
      >
        {status === "booking" ? "Reserving…" : "Confirm reservation"}
      </button>
      <p className="mt-2 text-center text-[12px] text-zinc-400">
        No deposit — we just hold your table.
      </p>
    </form>
  );
}

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20";

/* ------------------------------ labels -------------------------------- */

function labelDow(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { weekday: "short" });
}
function labelDay(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
function labelTime(iso: string) {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
// Day KEY ("YYYY-MM-DD") parsed LOCALLY — `new Date("YYYY-MM-DD")` is UTC and
// shifts back a day in negative-UTC zones.
function keyToLocalDate(key: string): Date {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d);
}
function dowOfKey(key: string) {
  return keyToLocalDate(key).toLocaleDateString(undefined, { weekday: "short" });
}
function dayOfKey(key: string) {
  return keyToLocalDate(key).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/* ----------------------------- skeletons ------------------------------ */

function PickerSkeleton() {
  return (
    <div className="rounded-2xl border border-zinc-200 bg-white p-7">
      <div className="flex gap-2">
        {[0, 1, 2, 3, 4].map((i) => (
          <div key={i} className="h-14 w-16 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <SeatingsSkeleton />
    </div>
  );
}

function SeatingsSkeleton() {
  return (
    <div className="mt-5 grid grid-cols-3 gap-2 sm:grid-cols-4">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="h-12 animate-pulse rounded-lg bg-zinc-100" />
      ))}
    </div>
  );
}
