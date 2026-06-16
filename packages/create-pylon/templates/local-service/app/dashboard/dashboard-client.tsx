"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { siteConfig } from "@/lib/site.config";
import type { BookingRow, OwnerBookingsResult } from "@/lib/booking";

// The owner's live bookings dashboard. Liveness rides the SAME public
// BookedSlot projection the picker uses: `db.useQuery("BookedSlot")` re-renders
// the instant a slot is taken or freed (cross-tab, via the replica), which
// triggers a re-fetch of the owner-gated `bookingsForOwner` — so new bookings
// appear and cancellations drop off without a refresh. Customer PII only ever
// travels through that gated call.
export function BookingsDashboard({ userEmail }: { userEmail: string }) {
  const { data: slotRows } = db.useQuery<{ id: string }>("BookedSlot");
  const liveKey = slotRows.length;

  const [bookings, setBookings] = useState<BookingRow[] | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  async function load() {
    try {
      const r = await callFn<OwnerBookingsResult>("bookingsForOwner", {});
      if (!r.authorized) {
        setDenied(true);
      } else {
        setBookings(r.bookings);
        setDenied(false);
        setError(null);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  useEffect(() => {
    void load();
    // Re-fetch whenever the live booked-slot set changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveKey]);

  async function act(id: string, fn: "confirmBooking" | "cancelBooking") {
    setBusyId(id);
    try {
      await callFn(fn, { bookingId: id });
      await load();
    } finally {
      setBusyId(null);
    }
  }

  if (denied) return <OwnerOnly email={userEmail} />;
  if (error) {
    return (
      <div className="rounded-xl border border-red-200 bg-red-50 px-5 py-4 text-sm text-red-700">
        {error}
      </div>
    );
  }
  if (!bookings) return <Skeleton />;

  // Upcoming = not cancelled and ending in the future. Group by day.
  const nowMs = Date.now();
  const active = bookings.filter((b) => b.status !== "cancelled");
  const upcoming = active.filter((b) => Date.parse(b.endsAt) >= nowMs);
  const todayKey = new Date().toLocaleDateString();
  const todayCount = upcoming.filter(
    (b) => new Date(b.startsAt).toLocaleDateString() === todayKey,
  ).length;
  const pendingCount = upcoming.filter((b) => b.status === "pending").length;

  return (
    <div className="space-y-8">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Bookings</h1>
        <p className="mt-1 text-sm text-zinc-500">
          Live — new bookings appear the moment they happen; cancelling frees the
          slot on the site instantly.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Stat label="Upcoming" value={upcoming.length} />
        <Stat label="Today" value={todayCount} />
        <Stat label="Awaiting confirm" value={pendingCount} />
      </div>

      <UpcomingList bookings={upcoming} busyId={busyId} onAct={act} />
    </div>
  );
}

function Stat({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-xl border border-zinc-200 bg-white p-4">
      <div className="text-[11px] font-medium uppercase tracking-wide text-zinc-400">
        {label}
      </div>
      <div className="mt-1 text-2xl font-semibold tabular-nums text-zinc-900">{value}</div>
    </div>
  );
}

function UpcomingList({
  bookings,
  busyId,
  onAct,
}: {
  bookings: BookingRow[];
  busyId: string | null;
  onAct: (id: string, fn: "confirmBooking" | "cancelBooking") => void;
}) {
  // Group chronologically by calendar day.
  const groups = useMemo(() => {
    const m = new Map<string, BookingRow[]>();
    for (const b of bookings) {
      const key = new Date(b.startsAt).toLocaleDateString(undefined, {
        weekday: "long",
        month: "short",
        day: "numeric",
      });
      const arr = m.get(key) ?? [];
      arr.push(b);
      m.set(key, arr);
    }
    return Array.from(m.entries());
  }, [bookings]);

  if (bookings.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-zinc-300 p-10 text-center text-sm text-zinc-500">
        No upcoming bookings yet. Share your site — new bookings land here live.
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {groups.map(([day, items]) => (
        <div key={day}>
          <h2 className="mb-2 text-[13px] font-semibold text-zinc-900">{day}</h2>
          <div className="overflow-hidden rounded-xl border border-zinc-200">
            {items.map((b, i) => (
              <BookingItem
                key={b.id}
                booking={b}
                busy={busyId === b.id}
                onAct={onAct}
                last={i === items.length - 1}
              />
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function BookingItem({
  booking,
  busy,
  onAct,
  last,
}: {
  booking: BookingRow;
  busy: boolean;
  onAct: (id: string, fn: "confirmBooking" | "cancelBooking") => void;
  last: boolean;
}) {
  const service = siteConfig.services.items.find((s) => s.slug === booking.serviceSlug);
  return (
    <div
      className={
        "flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 " +
        (last ? "" : "border-b border-zinc-100")
      }
    >
      <div className="w-20 shrink-0 text-[14px] font-semibold tabular-nums text-zinc-900">
        {new Date(booking.startsAt).toLocaleTimeString(undefined, {
          hour: "numeric",
          minute: "2-digit",
        })}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[14px] font-medium text-zinc-900">
            {booking.customerName}
          </span>
          <StatusBadge status={booking.status} />
        </div>
        <div className="truncate text-[12.5px] text-zinc-500">
          {service?.name ?? booking.serviceSlug} · {booking.customerEmail}
          {booking.customerPhone ? ` · ${booking.customerPhone}` : ""}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {booking.status === "pending" ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => onAct(booking.id, "confirmBooking")}
            className="rounded-md bg-zinc-900 px-3 py-1.5 text-[12.5px] font-medium text-white transition-colors hover:bg-zinc-700 disabled:opacity-50"
          >
            {busy ? "…" : "Confirm"}
          </button>
        ) : null}
        <button
          type="button"
          disabled={busy}
          onClick={() => onAct(booking.id, "cancelBooking")}
          className="rounded-md border border-zinc-300 px-3 py-1.5 text-[12.5px] font-medium text-zinc-600 transition-colors hover:border-red-300 hover:text-red-600 disabled:opacity-50"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const tone =
    status === "confirmed"
      ? "bg-green-50 text-green-700"
      : status === "cancelled"
        ? "bg-zinc-100 text-zinc-500"
        : "bg-amber-50 text-amber-700";
  return (
    <span className={"rounded-full px-2 py-0.5 text-[10px] font-medium capitalize " + tone}>
      {status}
    </span>
  );
}

/* --------------------------- owner-only gate -------------------------- */

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="rounded-xl border border-dashed border-zinc-300 px-6 py-12 text-center">
      <h1 className="text-lg font-semibold">This dashboard is owner-only</h1>
      <p className="mx-auto mt-2 max-w-md text-sm text-zinc-500">
        You&apos;re signed in as{" "}
        <span className="font-medium text-zinc-700">{email || "this account"}</span>.
        Only the shop owner can see bookings. Set{" "}
        <code className="rounded bg-zinc-100 px-1.5 py-0.5 text-[12px]">
          PYLON_OWNER_EMAIL={email || "you@yourshop.com"}
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

/* ------------------------------ skeleton ------------------------------ */

function Skeleton() {
  return (
    <div className="space-y-8">
      <div className="h-6 w-32 animate-pulse rounded bg-zinc-100" />
      <div className="grid gap-4 sm:grid-cols-3">
        {[0, 1, 2].map((i) => (
          <div key={i} className="h-20 animate-pulse rounded-xl bg-zinc-100" />
        ))}
      </div>
      <div className="h-48 animate-pulse rounded-xl bg-zinc-100" />
    </div>
  );
}
