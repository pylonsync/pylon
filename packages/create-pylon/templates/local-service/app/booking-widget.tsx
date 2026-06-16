"use client";

import React, { useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { EnsureGuest } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import { slotsForDay, weekdayOf, localDateKey, type Slot } from "@/lib/slots";

// The live booking picker — the realtime heart of this template. It subscribes
// to the public, PII-free `BookedSlot` projection with `db.useQuery`, so the
// instant anyone books a time (this tab or another), that slot greys out for
// everyone with the page open. The server independently re-checks at insert
// time, so even a dead-heat double-click can't double-book.
//
// Wrapped in <EnsureGuest> so the sync connection (which the live query rides)
// is established for anonymous visitors. The guest session holds no PII, and
// the Booking table stays unreadable to it — the picker only ever reads busy
// time ranges, never a customer's name or email.

interface BookedSlotRow {
  id: string;
  serviceSlug: string;
  startsAt: string;
  endsAt: string;
}

export function BookingWidget() {
  return (
    <EnsureGuest fallback={<PickerSkeleton />}>
      <Picker />
    </EnsureGuest>
  );
}

function Picker() {
  const { services, booking } = siteConfig;
  const { data: busyRows, loading } = db.useQuery<BookedSlotRow>("BookedSlot");

  // Stamp "now" once so slot availability + the day list are stable across
  // re-renders (and only computed client-side, post-hydration).
  const [nowMs] = useState(() => Date.now());

  const openDays = useMemo(() => {
    const days: string[] = [];
    for (let i = 0; i < booking.daysAhead; i++) {
      const key = localDateKey(i, nowMs);
      if (booking.hours[weekdayOf(key)]) days.push(key);
    }
    return days;
  }, [booking.daysAhead, booking.hours, nowMs]);

  const [serviceSlug, setServiceSlug] = useState(services.items[0]?.slug ?? "");
  const [day, setDay] = useState(openDays[0] ?? "");
  const [selected, setSelected] = useState<Slot | null>(null);
  // The slot we just successfully booked — kept separate from `selected` so the
  // confirmation card persists even after the live update marks the slot taken.
  const [confirmed, setConfirmed] = useState<{ slot: Slot; serviceName: string } | null>(null);

  const service = services.items.find((s) => s.slug === serviceSlug) ?? services.items[0];

  const slots = useMemo(() => {
    if (!service || !day) return [];
    const hrs = booking.hours[weekdayOf(day)];
    if (!hrs) return [];
    return slotsForDay({
      dayISODate: day,
      open: hrs.open,
      close: hrs.close,
      slotMinutes: booking.slotMinutes,
      durationMin: service.durationMin,
      leadTimeHours: booking.leadTimeHours,
      busy: busyRows.map((b) => ({ startsAt: b.startsAt, endsAt: b.endsAt })),
      nowMs,
    });
    // busyRows identity changes on every sync push → slots recompute live.
  }, [service, day, booking, busyRows, nowMs]);

  // Clear an IN-PROGRESS selection if its slot just got taken out from under us
  // (someone else booked it while this visitor was filling the form). A slot we
  // successfully booked ourselves lives in `confirmed`, not `selected`, so this
  // never wipes our own confirmation.
  const selectedStillFree =
    selected && slots.find((s) => s.startsAt === selected.startsAt)?.available;
  if (selected && !selectedStillFree) {
    queueMicrotask(() => setSelected(null));
  }

  function pick(slot: Slot) {
    setConfirmed(null);
    setSelected(slot);
  }

  return (
    <div className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm sm:p-7">
      {/* Service picker */}
      <div className="flex flex-wrap gap-2">
        {services.items.map((s) => {
          const active = s.slug === serviceSlug;
          return (
            <button
              key={s.slug}
              type="button"
              onClick={() => {
                setServiceSlug(s.slug);
                setSelected(null);
                setConfirmed(null);
              }}
              className={
                "rounded-full border px-3.5 py-1.5 text-[13px] font-medium transition-colors " +
                (active
                  ? "border-zinc-900 bg-zinc-900 text-white"
                  : "border-zinc-300 text-zinc-700 hover:border-zinc-400")
              }
            >
              {s.name}
              <span className={active ? "ml-1.5 text-white/60" : "ml-1.5 text-zinc-400"}>
                {s.price} · {s.durationMin}m
              </span>
            </button>
          );
        })}
      </div>

      {/* Day picker */}
      <div className="mt-5 flex gap-2 overflow-x-auto pb-1">
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
                (active
                  ? "border-brand bg-brand-soft"
                  : "border-zinc-200 hover:border-zinc-300")
              }
            >
              <span className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">
                {dowOfKey(key)}
              </span>
              <span
                className={
                  "text-[15px] font-semibold " + (active ? "text-brand" : "text-zinc-900")
                }
              >
                {dayOfKey(key)}
              </span>
            </button>
          );
        })}
      </div>

      {/* Slots */}
      <div className="mt-5 border-t border-zinc-100 pt-5">
        {loading ? (
          <SlotsSkeleton />
        ) : slots.length === 0 ? (
          <p className="py-6 text-center text-sm text-zinc-500">
            Closed that day — pick another.
          </p>
        ) : (
          <div className="grid grid-cols-3 gap-2 sm:grid-cols-4">
            {slots.map((slot) => {
              const isSelected = selected?.startsAt === slot.startsAt;
              return (
                <button
                  key={slot.startsAt}
                  type="button"
                  disabled={!slot.available}
                  onClick={() => pick(slot)}
                  aria-pressed={isSelected}
                  className={
                    "rounded-lg border py-2 text-[13px] font-medium tabular-nums transition-colors " +
                    (!slot.available
                      ? "cursor-not-allowed border-zinc-100 bg-zinc-50 text-zinc-300 line-through"
                      : isSelected
                        ? "border-brand bg-brand text-white"
                        : "border-zinc-200 text-zinc-800 hover:border-brand hover:text-brand")
                  }
                >
                  {labelTime(slot.startsAt)}
                </button>
              );
            })}
          </div>
        )}
        <p className="mt-3 text-[12px] text-zinc-400">
          Greyed-out times are already booked — this updates live as others book.
        </p>
      </div>

      {/* Confirmation persists after a successful booking… */}
      {confirmed ? (
        <div className="mt-6 rounded-xl border border-brand/30 bg-brand-soft/60 p-5 text-center">
          <p className="text-[15px] font-semibold text-zinc-900">
            {confirmed.serviceName} · {labelDow(confirmed.slot.startsAt)}{" "}
            {labelDay(confirmed.slot.startsAt)} at {labelTime(confirmed.slot.startsAt)}
          </p>
          <p className="mt-2 text-[14px] text-zinc-600">
            {siteConfig.booking.confirmationMessage}
          </p>
        </div>
      ) : selected && service ? (
        /* …otherwise the form for the chosen slot. */
        <BookingForm
          service={service}
          slot={selected}
          onClear={() => setSelected(null)}
          onBooked={() => {
            setConfirmed({ slot: selected, serviceName: service.name });
            setSelected(null);
          }}
        />
      ) : null}
    </div>
  );
}

function BookingForm({
  service,
  slot,
  onClear,
  onBooked,
}: {
  service: { slug: string; name: string; price: string };
  slot: Slot;
  onClear: () => void;
  onBooked: () => void;
}) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [phone, setPhone] = useState("");
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
      const res = await callFn<{ ok: boolean; reason?: string }>("createBooking", {
        serviceSlug: service.slug,
        startsAt: slot.startsAt,
        customerName: name.trim(),
        customerEmail: email.trim(),
        customerPhone: phone.trim() || undefined,
      });
      if (res.ok) {
        onBooked();
      } else {
        setStatus("idle");
        setError(
          res.reason === "taken"
            ? "Someone just grabbed that time — pick another."
            : res.reason === "past"
              ? "That's too soon — choose a later time."
              : "Couldn't book that slot. Try another.",
        );
        if (res.reason === "taken") onClear();
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
          {service.name} · {labelDow(slot.startsAt)} {labelDay(slot.startsAt)} at{" "}
          {labelTime(slot.startsAt)}
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
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Your name"
          aria-label="Your name"
          autoComplete="name"
          className={inputCls}
        />
        <input
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@email.com"
          aria-label="Email"
          autoComplete="email"
          className={inputCls}
        />
        <input
          type="tel"
          value={phone}
          onChange={(e) => setPhone(e.target.value)}
          placeholder="Phone (optional)"
          aria-label="Phone"
          autoComplete="tel"
          className={inputCls + " sm:col-span-2"}
        />
      </div>
      {error ? <p className="mt-3 text-[13px] text-red-600">{error}</p> : null}
      <button
        type="submit"
        disabled={status === "booking"}
        className="mt-4 inline-flex h-10 w-full items-center justify-center rounded-lg bg-brand text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-60"
      >
        {status === "booking" ? "Booking…" : "Confirm booking"}
      </button>
      <p className="mt-2 text-center text-[12px] text-zinc-400">
        No payment now — pay at the shop.
      </p>
    </form>
  );
}

const inputCls =
  "h-10 w-full rounded-lg border border-zinc-300 bg-white px-3 text-sm text-zinc-900 outline-none transition placeholder:text-zinc-400 focus:border-brand focus:ring-2 focus:ring-brand/20";

/* ------------------------------ labels -------------------------------- */

// For full ISO INSTANTS (a slot's startsAt) — `new Date(iso)` is the right
// instant, shown in the viewer's local time.
function labelDow(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { weekday: "short" });
}
function labelDay(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
function labelTime(iso: string) {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

// For a local DAY KEY ("YYYY-MM-DD") — parse the parts LOCALLY. `new Date("2026-06-15")`
// would parse as UTC midnight and shift back a day in negative-UTC zones, so
// build the date from its parts instead.
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
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="h-8 w-24 animate-pulse rounded-full bg-zinc-100" />
        ))}
      </div>
      <div className="mt-5 flex gap-2">
        {[0, 1, 2, 3, 4].map((i) => (
          <div key={i} className="h-14 w-16 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <SlotsSkeleton />
    </div>
  );
}

function SlotsSkeleton() {
  return (
    <div className="mt-5 grid grid-cols-3 gap-2 sm:grid-cols-4">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="h-9 animate-pulse rounded-lg bg-zinc-100" />
      ))}
    </div>
  );
}
