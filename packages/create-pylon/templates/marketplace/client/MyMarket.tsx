"use client";

import React, { useRef, useState } from "react";
import { Link, callFn, db } from "@pylonsync/react";
import { Check, Heart, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Spinner } from "@/components/ui/spinner";
import { listingSrcSet, percentOfAsk } from "../lib/catalog";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import {
  gradient,
  money,
  timeAgo,
  type Listing,
  type Offer,
  type Watch,
} from "./market";

function uniqueById<T extends { id: string }>(rows: T[]): T[] {
  return Array.from(new Map(rows.map((row) => [row.id, row])).values());
}

function Thumb({ listing, className }: { listing?: Listing; className?: string }) {
  return (
    <span
      className={cn("relative block shrink-0 overflow-hidden rounded-lg bg-muted", className)}
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
          sizes="64px"
          alt=""
          width="64"
          height="80"
          loading="lazy"
          className="size-full object-cover"
        />
      ) : null}
      <span aria-hidden="true" className="absolute inset-0 rounded-lg ring-1 ring-inset ring-black/5" />
    </span>
  );
}

function StatusTag({ status }: { status: string }) {
  return (
    <span
      className={cn(
        "shrink-0 rounded-full px-2 py-0.5 text-[11px] font-semibold capitalize",
        status === "pending" && "bg-signal/12 text-signal-foreground",
        (status === "accepted" || status === "sold") && "bg-success/14 text-success-foreground",
        status === "declined" && "bg-muted text-muted-foreground",
        status === "active" && "bg-muted text-foreground",
      )}
    >
      {status === "active" ? "Listed" : status}
    </span>
  );
}

function Section({
  title,
  count,
  children,
  className,
}: {
  title: string;
  count?: number;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("flex flex-col gap-3", className)}>
      <h2 className="flex items-baseline gap-2 text-base font-semibold">
        {title}
        {count !== undefined ? (
          <span className="text-sm font-normal tabular-nums text-muted-foreground">
            {count}
          </span>
        ) : null}
      </h2>
      {children}
    </section>
  );
}

function EmptyNote({ children }: { children: React.ReactNode }) {
  return (
    <p className="rounded-2xl bg-muted/60 px-4 py-5 text-sm text-muted-foreground">
      {children}
    </p>
  );
}

function Dashboard() {
  // Rendered inside <AuthGate>, so identity is non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";

  // Three live queries. Listings and offers are public-read and narrowed to
  // this user in memory; Watch is policy-scoped to its owner. A new offer on
  // one of your listings, or a seller answering yours, updates in place.
  const { data: listings } = db.useQuery<Listing>("Listing", {
    orderBy: { createdAt: "desc" },
  });
  const { data: offers, loading: offersLoading } = db.useQuery<Offer>("Offer", {
    orderBy: { createdAt: "desc" },
  });
  const { data: watching } = db.useQuery<Watch>("Watch", {
    where: { userId },
    orderBy: { createdAt: "desc" },
  });

  const allListings = uniqueById(listings ?? []);
  const listingById = new Map(allListings.map((l) => [l.id, l]));
  const myListings = allListings.filter((l) => l.sellerId === userId);
  const allOffers = uniqueById(offers ?? []);
  const myOffers = allOffers.filter((o) => o.buyerId === userId);
  const inbound = allOffers.filter((o) => o.sellerId === userId);
  const waiting = inbound.filter((o) => o.status === "pending");
  const watchlist = uniqueById(watching ?? []);
  const pendingFor = (listingId: string) =>
    waiting.filter((o) => o.listingId === listingId).length;

  const seen = useRef<Set<string> | null>(null);
  if (seen.current === null && !offersLoading) {
    seen.current = new Set(allOffers.map((o) => o.id));
  }
  const isNew = (id: string) => seen.current !== null && !seen.current.has(id);

  const stats: Array<[string, number]> = [
    ["Listed", myListings.filter((l) => l.status === "active").length],
    ["Offers waiting", waiting.length],
    ["Offers sent", myOffers.length],
    ["Watching", watchlist.length],
  ];

  return (
    <div className="flex flex-col gap-10 pb-6 pt-8 sm:pt-10">
      <header className="flex flex-col items-start justify-between gap-5 sm:flex-row sm:items-end">
        <div>
          <h1 className="font-display text-[44px] leading-none sm:text-[52px]">
            Your market
          </h1>
          <p className="mt-3 text-sm text-muted-foreground">
            Signed in as <span className="font-medium text-foreground">{name}</span>.
            Offers and sales update here live.
          </p>
        </div>
        <Link
          href="/sell"
          className="inline-flex min-h-11 items-center rounded-full bg-primary px-5 text-sm font-medium text-primary-foreground"
        >
          Sell an item
        </Link>
      </header>

      <dl className="grid grid-cols-2 gap-px overflow-hidden rounded-2xl bg-border shadow-[var(--shadow-border)] sm:grid-cols-4">
        {stats.map(([label, value]) => (
          <div key={label} className="bg-card px-5 py-4">
            <dt className="text-[13px] text-muted-foreground">{label}</dt>
            <dd className="mt-1 font-display text-[40px] leading-none tabular-nums">
              {value}
            </dd>
          </div>
        ))}
      </dl>

      <Section title="Offers waiting for you" count={waiting.length}>
        {waiting.length === 0 ? (
          <EmptyNote>
            No offers waiting. When a buyer sends one, it appears here at once.
          </EmptyNote>
        ) : (
          <ul className="flex flex-col gap-2" aria-live="polite">
            {waiting.map((o) => (
              <InboundOffer
                key={o.id}
                offer={o}
                listing={listingById.get(o.listingId)}
                fresh={isNew(o.id)}
              />
            ))}
          </ul>
        )}
      </Section>

      <div className="grid gap-10 lg:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)] lg:gap-12">
        <Section title="Your listings" count={myListings.length}>
          {myListings.length === 0 ? (
            <EmptyNote>
              Nothing listed yet.{" "}
              <Link href="/sell" className="font-medium text-foreground underline underline-offset-4">
                Post your first item
              </Link>
              .
            </EmptyNote>
          ) : (
            <ul className="grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-4">
              {myListings.map((l) => (
                <li key={l.id}>
                  <Link href={`/listing/${l.slug || l.id}`} seed={l} className="group block">
                    <span className="relative block">
                      <Thumb listing={l} className="aspect-[4/5] w-full rounded-xl" />
                      {l.status === "sold" ? (
                        <span className="absolute left-2 top-2 rounded-full bg-foreground px-2 py-0.5 text-[11px] font-semibold text-background">
                          Sold
                        </span>
                      ) : pendingFor(l.id) > 0 ? (
                        <span className="absolute left-2 top-2 inline-flex items-center gap-1 rounded-full bg-white/92 px-2 py-0.5 text-[11px] font-medium text-neutral-900">
                          <span className="size-1.5 rounded-full bg-signal" aria-hidden="true" />
                          {pendingFor(l.id)} {pendingFor(l.id) === 1 ? "offer" : "offers"}
                        </span>
                      ) : null}
                    </span>
                    <span className="mt-2 block text-sm font-semibold tabular-nums">
                      {money(l.price)}
                    </span>
                    <span className="block truncate text-[13px] text-muted-foreground group-hover:text-foreground">
                      {l.title}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </Section>

        <div className="flex flex-col gap-10">
          <Section title="Offers you sent" count={myOffers.length}>
            {myOffers.length === 0 ? (
              <EmptyNote>
                You have not made an offer.{" "}
                <Link href="/" className="font-medium text-foreground underline underline-offset-4">
                  Browse listings
                </Link>
                .
              </EmptyNote>
            ) : (
              <ul className="flex flex-col rounded-2xl bg-card shadow-[var(--shadow-border)]">
                {myOffers.map((o) => {
                  const listing = listingById.get(o.listingId);
                  return (
                    <li key={o.id} className={cn("border-b border-border/60 last:border-b-0", isNew(o.id) && "market-flash")}>
                      <Link
                        href={`/listing/${listing?.slug || o.listingId}`}
                        seed={listing}
                        className="flex items-center gap-3 p-3"
                      >
                        <Thumb listing={listing} className="h-12 w-10" />
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium">{o.listingTitle}</span>
                          <span className="block text-xs text-muted-foreground">
                            {timeAgo(o.createdAt)}
                          </span>
                        </span>
                        <span className="text-sm font-semibold tabular-nums">{money(o.amount)}</span>
                        <StatusTag status={o.status} />
                      </Link>
                    </li>
                  );
                })}
              </ul>
            )}
          </Section>

          <Section title="Watching" count={watchlist.length}>
            {watchlist.length === 0 ? (
              <EmptyNote>
                Select the <Heart aria-hidden="true" className="inline size-3.5 align-[-2px]" /> on
                any listing to save it here. Only you can see this list.
              </EmptyNote>
            ) : (
              <ul className="flex flex-col rounded-2xl bg-card shadow-[var(--shadow-border)]">
                {watchlist.map((w) => {
                  const listing = listingById.get(w.listingId);
                  return (
                    <li key={w.id} className="border-b border-border/60 last:border-b-0">
                      <Link
                        href={`/listing/${listing?.slug || w.listingId}`}
                        seed={listing}
                        className="flex items-center gap-3 p-3"
                      >
                        <Thumb listing={listing} className="h-12 w-10" />
                        <span className="min-w-0 flex-1 truncate text-sm font-medium">
                          {w.listingTitle}
                        </span>
                        {listing ? (
                          <span className="text-sm font-semibold tabular-nums">
                            {money(listing.price)}
                          </span>
                        ) : null}
                        {listing?.status === "sold" ? <StatusTag status="sold" /> : null}
                      </Link>
                    </li>
                  );
                })}
              </ul>
            )}
          </Section>
        </div>
      </div>
    </div>
  );
}

function InboundOffer({
  offer,
  listing,
  fresh,
}: {
  offer: Offer;
  listing?: Listing;
  fresh: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function respond(accept: boolean) {
    setBusy(true);
    setErr(null);
    try {
      await callFn("respondToOffer", { offerId: offer.id, accept });
    } catch (e) {
      setErr((e as Error).message ?? "Could not answer the offer.");
      setBusy(false);
      setConfirming(false);
    }
  }

  return (
    <li
      className={cn(
        "flex flex-wrap items-center gap-4 rounded-2xl bg-card p-3 pr-4 shadow-[var(--shadow-border)]",
        fresh && "market-rise market-flash",
      )}
    >
      <Link
        href={`/listing/${listing?.slug || offer.listingId}`}
        seed={listing}
        className="flex min-w-0 flex-1 items-center gap-3"
      >
        <Thumb listing={listing} className="h-16 w-[52px]" />
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium">{offer.listingTitle}</span>
          <span className="block truncate text-[13px] text-muted-foreground">
            {offer.buyerName} · {timeAgo(offer.createdAt)}
            {offer.message ? ` · “${offer.message}”` : ""}
          </span>
        </span>
      </Link>
      <span className="text-right">
        <span className="block text-lg font-semibold leading-tight tabular-nums">
          {money(offer.amount)}
        </span>
        {listing ? (
          <span className="block text-[11px] tabular-nums text-muted-foreground">
            {percentOfAsk(offer.amount, listing.price)}% of {money(listing.price)}
          </span>
        ) : null}
      </span>
      <span className="flex shrink-0 gap-2">
        {confirming ? (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => respond(true)}
              className="inline-flex min-h-9 items-center gap-1.5 rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground disabled:opacity-60"
            >
              {busy ? <Spinner /> : <Check className="size-3.5" />}
              Sell for {money(offer.amount)}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(false)}
              className="inline-flex min-h-9 items-center rounded-full px-3 text-[13px] font-medium text-muted-foreground hover:text-foreground"
            >
              Cancel
            </button>
          </>
        ) : (
          <>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(true)}
              className="inline-flex min-h-9 items-center rounded-full bg-primary px-3.5 text-[13px] font-medium text-primary-foreground transition-[scale] active:scale-[0.97] disabled:opacity-60"
            >
              Accept
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => respond(false)}
              className="inline-flex min-h-9 items-center gap-1 rounded-full bg-background px-3 text-[13px] font-medium shadow-[var(--shadow-border)] transition-[scale] active:scale-[0.97] disabled:opacity-60"
            >
              <X className="size-3.5" />
              Decline
            </button>
          </>
        )}
      </span>
      {err ? <p className="w-full text-sm text-destructive">{err}</p> : null}
    </li>
  );
}

export function MyMarket() {
  return (
    <MarketProvider
      fallback={
        <div className="flex items-center gap-2 pt-10 text-sm text-muted-foreground">
          <Spinner />
          Loading your market
        </div>
      }
    >
      <AuthGate
        title="Sign in to open your dashboard"
        blurb="Your listings, offers, and saved items stay in sync here. The demo account is filled in."
        headingLevel={1}
      >
        <Dashboard />
      </AuthGate>
    </MarketProvider>
  );
}
