"use client";

import React, { useRef } from "react";
import { Link, db } from "@pylonsync/react";
import { cn } from "@/lib/utils";
import { listingSrcSet } from "../lib/catalog";
import { MarketProvider } from "./MarketProvider";
import { gradient, money, timeAgo, type Listing, type Offer } from "./market";

// Live marketplace activity: new listings, offers, and sales, merged from two
// live queries. A row written in any tab (by any visitor) slides in at the
// top with a short highlight. Reads are public, so this runs for signed-out
// visitors on the guest session.

type ActivityEvent = {
  key: string;
  kind: "listed" | "offer" | "sold";
  at: string;
  listing?: Listing;
  title: string;
  href: string;
  who: string;
  amount: number;
};

function useEvents(limit: number): {
  events: ActivityEvent[];
  forSale: number;
  openOffers: number;
  loading: boolean;
} {
  const { data: listingRows, loading } = db.useQuery<Listing>("Listing", {
    orderBy: { createdAt: "desc" },
  });
  const { data: offerRows } = db.useQuery<Offer>("Offer", {
    orderBy: { createdAt: "desc" },
    limit: 40,
  });
  const listings = listingRows ?? [];
  const offers = offerRows ?? [];
  const byId = new Map(listings.map((l) => [l.id, l]));

  const events: ActivityEvent[] = [
    ...listings.slice(0, 20).map((l) => ({
      key: `l-${l.id}`,
      kind: "listed" as const,
      at: l.createdAt,
      listing: l,
      title: l.title,
      href: `/listing/${l.slug || l.id}`,
      who: l.sellerName,
      amount: l.price,
    })),
    ...offers.map((o) => {
      const listing = byId.get(o.listingId);
      return {
        key: `o-${o.id}`,
        kind: o.status === "accepted" ? ("sold" as const) : ("offer" as const),
        at: o.createdAt,
        listing,
        title: o.listingTitle,
        href: `/listing/${listing?.slug || o.listingId}`,
        who: o.buyerName,
        amount: o.amount,
      };
    }),
  ]
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, limit);

  return {
    events,
    forSale: listings.filter((l) => l.status === "active").length,
    openOffers: offers.filter((o) => o.status === "pending").length,
    loading,
  };
}

function Thumb({ listing, className }: { listing?: Listing; className?: string }) {
  return (
    <span
      className={cn(
        "relative block shrink-0 overflow-hidden rounded-lg bg-muted",
        className,
      )}
      style={
        listing && !listing.imageUrl
          ? { background: gradient(listing.seed || listing.id) }
          : undefined
      }
    >
      {listing?.imageUrl ? (
        <img
          src={listing.imageUrl}
          srcSet={listingSrcSet(listing.imageUrl)}
          sizes="48px"
          alt=""
          width="48"
          height="60"
          loading="lazy"
          className="size-full object-cover"
        />
      ) : null}
      <span
        aria-hidden="true"
        className="absolute inset-0 rounded-lg ring-1 ring-inset ring-black/5"
      />
    </span>
  );
}

function Sentence({ event }: { event: ActivityEvent }) {
  if (event.kind === "listed") {
    return (
      <>
        <span className="font-medium text-foreground">{event.who}</span> listed{" "}
        <span className="text-foreground">{event.title}</span>
      </>
    );
  }
  if (event.kind === "sold") {
    return (
      <>
        <span className="text-foreground">{event.title}</span> sold for{" "}
        <span className="font-medium tabular-nums text-foreground">
          {money(event.amount)}
        </span>
      </>
    );
  }
  return (
    <>
      <span className="font-medium text-foreground">{event.who}</span> offered{" "}
      <span className="font-medium tabular-nums text-foreground">
        {money(event.amount)}
      </span>{" "}
      on <span className="text-foreground">{event.title}</span>
    </>
  );
}

function Tag({ kind }: { kind: ActivityEvent["kind"] }) {
  const label = { listed: "Listed", offer: "Offer", sold: "Sold" }[kind];
  return (
    <span
      className={cn(
        "rounded-full px-1.5 py-px text-[10px] font-semibold",
        kind === "offer" && "bg-signal/12 text-signal-foreground",
        kind === "sold" && "bg-foreground text-background",
        kind === "listed" && "bg-muted text-muted-foreground",
      )}
    >
      {label}
    </span>
  );
}

function LiveDot() {
  return (
    <span className="relative flex size-2" aria-hidden="true">
      <span className="market-ping absolute inline-flex size-full rounded-full bg-signal" />
      <span className="relative inline-flex size-2 rounded-full bg-signal" />
    </span>
  );
}

function Rail() {
  const { events, forSale, openOffers, loading } = useEvents(9);
  // Keys rendered on the first settled pass. Later keys arrived live.
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null && !loading) {
    seen.current = new Set(events.map((e) => e.key));
  }

  return (
    <section
      aria-labelledby="activity-heading"
      className="rounded-2xl bg-card shadow-[var(--shadow-border)]"
    >
      <div className="flex items-center justify-between px-4 pb-3 pt-4">
        <h2
          id="activity-heading"
          className="flex items-center gap-2 text-sm font-semibold"
        >
          <LiveDot />
          Live activity
        </h2>
      </div>
      <dl className="mx-4 grid grid-cols-2 gap-px overflow-hidden rounded-xl bg-border">
        <div className="bg-card px-3 py-2.5">
          <dt className="text-xs text-muted-foreground">For sale</dt>
          <dd className="font-display text-[28px] leading-8 tabular-nums">
            {forSale}
          </dd>
        </div>
        <div className="bg-card px-3 py-2.5">
          <dt className="text-xs text-muted-foreground">Open offers</dt>
          <dd className="font-display text-[28px] leading-8 tabular-nums">
            {openOffers}
          </dd>
        </div>
      </dl>
      <ol aria-live="polite" className="mt-2 flex flex-col px-2 pb-2">
        {events.map((event) => {
          const fresh = seen.current !== null && !seen.current.has(event.key);
          return (
            <li key={event.key} className={cn(fresh && "market-rise")}>
              <Link
                href={event.href}
                seed={event.listing}
                className={cn(
                  "flex items-start gap-3 rounded-xl px-2 py-2 transition-colors hover:bg-accent",
                  fresh && "market-flash",
                )}
              >
                <Thumb listing={event.listing} className="h-[50px] w-10" />
                <span className="min-w-0 flex-1">
                  <span className="line-clamp-2 text-[13px] leading-[18px] text-muted-foreground">
                    <Sentence event={event} />
                  </span>
                  <span className="mt-1 flex items-center gap-2">
                    <Tag kind={event.kind} />
                    <span className="text-[11px] text-muted-foreground">
                      {timeAgo(event.at)}
                    </span>
                  </span>
                </span>
              </Link>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function Strip() {
  const { events, loading } = useEvents(8);
  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null && !loading) {
    seen.current = new Set(events.map((e) => e.key));
  }
  if (events.length === 0) return null;

  return (
    <section aria-label="Live activity" className="-mx-4 sm:-mx-8">
      <ol className="hide-scrollbar flex gap-2 overflow-x-auto px-4 sm:px-8">
        <li className="flex shrink-0 items-center gap-2 pr-1 text-xs font-semibold">
          <LiveDot />
          Live
        </li>
        {events.map((event) => {
          const fresh = seen.current !== null && !seen.current.has(event.key);
          return (
            <li key={event.key} className={cn("shrink-0", fresh && "market-rise")}>
              <Link
                href={event.href}
                seed={event.listing}
                className={cn(
                  "flex w-64 items-center gap-2.5 rounded-xl bg-card p-1.5 pr-3 shadow-[var(--shadow-border)]",
                  fresh && "market-flash",
                )}
              >
                <Thumb listing={event.listing} className="h-10 w-8" />
                <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                  <Sentence event={event} />
                </span>
              </Link>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

export function ActivityRail() {
  return (
    <MarketProvider fallback={<RailSkeleton />}>
      <Rail />
    </MarketProvider>
  );
}

export function ActivityStrip() {
  return (
    <MarketProvider fallback={<div className="h-[52px]" />}>
      <Strip />
    </MarketProvider>
  );
}

function RailSkeleton() {
  return (
    <div className="rounded-2xl bg-card p-4 shadow-[var(--shadow-border)]">
      <div className="h-4 w-28 rounded bg-muted" />
      <div className="mt-4 h-16 rounded-xl bg-muted" />
      <div className="mt-4 flex flex-col gap-3">
        {Array.from({ length: 6 }).map((_, i) => (
          <div key={i} className="flex gap-3">
            <div className="h-[50px] w-10 rounded-lg bg-muted" />
            <div className="flex flex-1 flex-col gap-2 pt-1">
              <div className="h-3 rounded bg-muted" />
              <div className="h-3 w-1/2 rounded bg-muted" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
