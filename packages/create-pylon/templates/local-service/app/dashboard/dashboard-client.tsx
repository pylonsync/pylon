"use client";

import React, { useEffect, useMemo, useState } from "react";
import { db, callFn } from "@pylonsync/react";
import { useAuth } from "@pylonsync/client";
import { Check, LogOut, MapPin, X } from "lucide-react";
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

  // In `pylon dev` an empty booking book gets demo appointments
  // (functions/seedDemo.ts). On a deploy the call returns without writing.
  useEffect(() => {
    void callFn("seedDemo", {}).catch(() => {});
  }, []);

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
    return <div className="border border-brand bg-brand-soft px-5 py-4 text-sm text-brand">{error}</div>;
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
  const weekEnd = nowMs + 7 * 86_400_000;
  const weekRevenue = upcoming
    .filter((b) => Date.parse(b.startsAt) < weekEnd)
    .reduce((sum, b) => sum + priceOf(b.serviceSlug), 0);

  return (
    <div className="space-y-10">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="font-display text-[3.5rem] leading-none sm:text-[4.5rem]">Bookings</h1>
          <p className="mt-3 max-w-lg text-[15px] leading-relaxed text-ink/70">
            New bookings appear here as they are made. Cancelling a booking frees the time on
            the site right away.
          </p>
        </div>
        <p className="flex items-center gap-2 text-[13px] font-medium">
          <span className="relative flex size-2">
            <span className="absolute inline-flex size-2 animate-ping rounded-full bg-green-500/60" />
            <span className="relative inline-flex size-2 rounded-full bg-green-600" />
          </span>
          Live
        </p>
      </div>

      <dl className="grid grid-cols-2 gap-px border border-ink bg-ink sm:grid-cols-4">
        <Stat label="Upcoming" value={String(upcoming.length)} />
        <Stat label="Today" value={String(todayCount)} />
        <Stat label="To confirm" value={String(pendingCount)} accent={pendingCount > 0} />
        <Stat label="Booked, next 7 days" value={`$${weekRevenue.toLocaleString("en-US")}`} />
      </dl>

      <UpcomingList bookings={upcoming} busyId={busyId} onAct={act} />

      {!siteConfig.location.mapEmbedUrl ? <MapHint /> : null}
    </div>
  );
}

function priceOf(slug: string): number {
  return siteConfig.services.items.find((s) => s.slug === slug)?.priceUsd ?? 0;
}

function Stat({ label, value, accent }: { label: string; value: string; accent?: boolean }) {
  return (
    <div className="bg-cream px-4 py-4">
      <dt className="text-[11px] font-medium uppercase tracking-wide text-ink/60">{label}</dt>
      <dd
        className={
          "mt-1 font-display text-[2.75rem] leading-none tabular-nums " + (accent ? "text-brand" : "")
        }
      >
        {value}
      </dd>
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
      const key = new Date(b.startsAt).toLocaleDateString("en-US", {
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
      <div className="border border-dashed border-ink/50 p-10 text-center text-[15px] text-ink/70">
        No upcoming bookings. New bookings appear here as soon as someone books a chair.
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {groups.map(([day, items]) => (
        <section key={day}>
          <div className="flex items-baseline justify-between border-b border-ink pb-2">
            <h2 className="font-display text-[2rem] leading-none">{day}</h2>
            <span className="text-[13px] text-ink/60">
              {items.length} {items.length === 1 ? "booking" : "bookings"}
            </span>
          </div>
          <ul className="divide-y divide-ink/20">
            {items.map((b) => (
              <BookingItem key={b.id} booking={b} busy={busyId === b.id} onAct={onAct} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function BookingItem({
  booking,
  busy,
  onAct,
}: {
  booking: BookingRow;
  busy: boolean;
  onAct: (id: string, fn: "confirmBooking" | "cancelBooking") => void;
}) {
  const service = siteConfig.services.items.find((s) => s.slug === booking.serviceSlug);
  return (
    <li className="flex flex-wrap items-center gap-x-5 gap-y-2 py-4">
      <div className="w-24 shrink-0 font-display text-[1.75rem] leading-none tabular-nums">
        {new Date(booking.startsAt).toLocaleTimeString("en-US", {
          hour: "numeric",
          minute: "2-digit",
        })}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[15px] font-semibold">{booking.customerName}</span>
          <StatusTag status={booking.status} />
        </div>
        <div className="mt-0.5 truncate text-[13px] text-ink/65">
          {service ? `${service.name} · ${service.durationMin} min · ${service.price}` : booking.serviceSlug}
          {" · "}
          {booking.customerPhone ?? booking.customerEmail}
        </div>
      </div>
      <div className="flex items-center gap-2">
        {booking.status === "pending" ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => onAct(booking.id, "confirmBooking")}
            className="inline-flex h-9 items-center gap-1.5 bg-ink px-3.5 text-[13px] font-medium text-cream transition-opacity hover:opacity-85 disabled:opacity-50"
          >
            <Check aria-hidden className="size-3.5" />
            Confirm
          </button>
        ) : null}
        <button
          type="button"
          disabled={busy}
          onClick={() => onAct(booking.id, "cancelBooking")}
          className="inline-flex h-9 items-center gap-1.5 border border-ink/40 px-3.5 text-[13px] font-medium text-ink/75 transition-colors hover:border-brand hover:text-brand disabled:opacity-50"
        >
          <X aria-hidden className="size-3.5" />
          Cancel
        </button>
      </div>
    </li>
  );
}

function StatusTag({ status }: { status: string }) {
  const tone =
    status === "confirmed"
      ? "border-ink/30 text-ink/70"
      : status === "cancelled"
        ? "border-ink/20 text-ink/40"
        : "border-brand bg-brand-soft text-brand";
  const label = status === "pending" ? "To confirm" : status === "confirmed" ? "Confirmed" : "Cancelled";
  return <span className={"border px-1.5 py-px text-[11px] font-medium " + tone}>{label}</span>;
}

// Setup notes for the owner live here, not on the public site.
function MapHint() {
  return (
    <aside className="flex gap-3 border border-ink/30 bg-paper/60 p-4 text-[13.5px] leading-relaxed text-ink/75">
      <MapPin aria-hidden className="mt-0.5 size-4 shrink-0 text-ink" />
      <p>
        Your site shows the weekly hours next to the address. To show a map there instead, paste a
        Google Maps embed URL into <code className="bg-cream px-1 text-[12.5px]">location.mapEmbedUrl</code> in{" "}
        <code className="bg-cream px-1 text-[12.5px]">lib/site.config.ts</code>.
      </p>
    </aside>
  );
}

/* --------------------------- owner-only gate -------------------------- */

function OwnerOnly({ email }: { email: string }) {
  return (
    <div className="border border-dashed border-ink/50 px-6 py-12 text-center">
      <h1 className="font-display text-[2.5rem] leading-none">This dashboard is owner-only</h1>
      <p className="mx-auto mt-4 max-w-md text-[14px] leading-relaxed text-ink/70">
        You are signed in as <span className="font-medium text-ink">{email || "this account"}</span>.
        Only the shop owner can see bookings. Set{" "}
        <code className="bg-paper px-1.5 py-0.5 text-[12px]">
          PYLON_OWNER_EMAIL={email || "you@yourshop.com"}
        </code>{" "}
        in <code className="bg-paper px-1.5 py-0.5 text-[12px]">.env</code>, restart, and reload. Or
        sign in with the owner account.
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
        className="flex size-9 cursor-pointer select-none list-none items-center justify-center bg-brand font-display text-[20px] leading-none text-cream marker:hidden [&::-webkit-details-marker]:hidden"
      >
        {initial}
      </summary>
      <div className="absolute right-0 top-full z-40 mt-2 w-60 border border-ink bg-cream py-1">
        <div className="truncate border-b border-ink/20 px-3 py-2.5 text-[13px] font-medium">
          {email || "Signed in"}
        </div>
        <button
          type="button"
          onClick={onSignOut}
          className="flex w-full items-center gap-2 px-3 py-2 text-left text-[13px] text-ink/75 transition-colors hover:bg-paper hover:text-ink"
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
      <div className="h-16 w-64 animate-pulse bg-ink/10" />
      <div className="h-24 animate-pulse bg-ink/10" />
      <div className="h-72 animate-pulse bg-ink/10" />
    </div>
  );
}
