"use client";

import React from "react";
import { Link, db } from "@pylonsync/react";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { Heart } from "lucide-react";
import { money, timeAgo, type Listing, type Offer, type Watch } from "./market";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "success" | "warning";

const statusVariant: Record<string, BadgeVariant> = {
  pending: "warning",
  accepted: "success",
  declined: "outline",
};

function Dashboard() {
  // Rendered inside <AuthGate>, so identity is non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";

  // Three live queries, all scoped to me. Everything updates in place: a new
  // offer on my listing, a seller answering my bid — no refresh.
  const { data: listings } = db.useQuery<Listing>("Listing", {
    where: { sellerId: userId },
    orderBy: { createdAt: "desc" },
  });
  const { data: received } = db.useQuery<Offer>("Offer", {
    where: { sellerId: userId },
    orderBy: { createdAt: "desc" },
  });
  const { data: watching } = db.useQuery<Watch>("Watch", {
    where: { userId },
    orderBy: { createdAt: "desc" },
  });
  const { data: sent } = db.useQuery<Offer>("Offer", {
    where: { buyerId: userId },
    orderBy: { createdAt: "desc" },
  });

  const myListings = listings ?? [];
  const myOffers = sent ?? [];
  const watchlist = watching ?? [];
  const inbound = received ?? [];
  const pendingFor = (listingId: string) =>
    inbound.filter((o) => o.listingId === listingId && o.status === "pending")
      .length;

  return (
    <div className="space-y-10 py-2 sm:py-6">
      <header className="flex flex-col items-start justify-between gap-5 sm:flex-row sm:items-end">
        <div>
          <h1 className="text-3xl font-semibold tracking-[-0.03em]">Dashboard</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Signed in as <span className="font-medium text-foreground">{name}</span>.
          </p>
        </div>
        <Button asChild>
          <Link href="/sell">Sell something</Link>
        </Button>
      </header>

      <section className="space-y-3">
        <h2 className="font-semibold">Your listings ({myListings.length})</h2>
        {myListings.length === 0 ? (
          <p className="rounded-2xl bg-card p-8 text-center text-sm text-muted-foreground shadow-[var(--shadow-border)]">
            Nothing listed yet.{" "}
            <Link href="/sell" className="underline">
              Post your first item
            </Link>
            .
          </p>
        ) : (
          <ul className="divide-y overflow-hidden rounded-2xl bg-card shadow-[var(--shadow-border)]">
            {myListings.map((l) => (
              <li key={l.id} className="flex items-center justify-between gap-3 p-3">
                <Link
                  href={`/listing/${l.slug || l.id}`}
                  className="flex min-w-0 items-center gap-3 hover:underline"
                >
                  {l.imageUrl ? (
                    <img
                      src={l.imageUrl}
                      alt=""
                      loading="lazy"
                      className="size-12 shrink-0 rounded-lg object-cover outline outline-1 -outline-offset-1 outline-black/10 dark:outline-white/10"
                    />
                  ) : null}
                  <span className="truncate font-medium">{l.title}</span>
                </Link>
                <div className="flex shrink-0 items-center gap-2 text-sm">
                  <span className="font-semibold">{money(l.price)}</span>
                  {l.status === "sold" ? (
                    <Badge variant="success">Sold</Badge>
                  ) : pendingFor(l.id) > 0 ? (
                    <Badge variant="warning">
                      {pendingFor(l.id)} offer{pendingFor(l.id) > 1 ? "s" : ""}
                    </Badge>
                  ) : (
                    <Badge variant="outline">Active</Badge>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-3">
        <h2 className="font-semibold">Offers you've made ({myOffers.length})</h2>
        {myOffers.length === 0 ? (
          <p className="rounded-2xl bg-card p-8 text-center text-sm text-muted-foreground shadow-[var(--shadow-border)]">
            No offers out.{" "}
            <Link href="/" className="underline">
              Browse the market
            </Link>
            .
          </p>
        ) : (
          <ul className="divide-y overflow-hidden rounded-2xl bg-card shadow-[var(--shadow-border)]">
            {myOffers.map((o) => (
              <li key={o.id} className="flex items-center justify-between gap-3 p-3">
                <Link
                  href={`/listing/${o.listingId}`}
                  className="min-w-0 hover:underline"
                >
                  <span className="truncate font-medium">{o.listingTitle}</span>
                  <span className="ml-2 text-sm text-muted-foreground">
                    {timeAgo(o.createdAt)}
                  </span>
                </Link>
                <div className="flex shrink-0 items-center gap-2 text-sm">
                  <span className="font-semibold tabular-nums">{money(o.amount)}</span>
                  <Badge variant={statusVariant[o.status] ?? "outline"}>
                    {o.status}
                  </Badge>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-3">
        <h2 className="flex items-center gap-2 font-semibold">
          <Heart className="size-4 fill-foreground text-foreground" />
          Watching ({watchlist.length})
        </h2>
        {watchlist.length === 0 ? (
          <p className="rounded-2xl bg-card p-8 text-center text-sm text-muted-foreground shadow-[var(--shadow-border)]">
            Nothing saved yet. Tap the{" "}
            <Heart className="inline size-3.5 align-text-bottom" /> on any
            listing to save it. Your watchlist is private and syncs live.
          </p>
        ) : (
          <ul className="divide-y overflow-hidden rounded-2xl bg-card shadow-[var(--shadow-border)]">
            {watchlist.map((w) => (
              <li key={w.id} className="flex items-center justify-between gap-3 p-3">
                <Link
                  href={`/listing/${w.listingId}`}
                  className="min-w-0 truncate font-medium hover:underline"
                >
                  {w.listingTitle}
                </Link>
                <span className="shrink-0 text-xs text-muted-foreground">
                  saved {timeAgo(w.createdAt)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

export function MyMarket() {
  return (
    <MarketProvider fallback={<p className="text-sm text-muted-foreground">Loading your market…</p>}>
      <AuthGate
        title="Sign in to open your dashboard"
        blurb="Your listings, saved finds, and offers stay synced here. The demo account is ready; just select Log in."
      >
        <Dashboard />
      </AuthGate>
    </MarketProvider>
  );
}
