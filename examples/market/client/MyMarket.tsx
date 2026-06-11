"use client";

import React from "react";
import { Link, db } from "@pylonsync/react";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { Heart } from "lucide-react";
import { money, timeAgo, type Listing, type Offer, type Watch } from "./market";

const statusBadge: Record<string, string> = {
  pending: "bg-amber-100 text-amber-800 border-amber-200",
  accepted: "bg-emerald-100 text-emerald-800 border-emerald-200",
  declined: "bg-zinc-100 text-zinc-500 border-zinc-200",
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
    <div className="space-y-10">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">My Market</h1>
          <p className="text-sm text-muted-foreground">
            You're trading as <span className="font-medium">{name}</span>.
          </p>
        </div>
        <Button asChild>
          <Link href="/sell">Sell something</Link>
        </Button>
      </header>

      <section className="space-y-3">
        <h2 className="font-semibold">Your listings ({myListings.length})</h2>
        {myListings.length === 0 ? (
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
            Nothing listed yet.{" "}
            <Link href="/sell" className="underline">
              Post your first item
            </Link>
            .
          </p>
        ) : (
          <ul className="divide-y rounded-lg border">
            {myListings.map((l) => (
              <li key={l.id} className="flex items-center justify-between gap-3 p-3">
                <Link href={`/listing/${l.slug || l.id}`} className="min-w-0 hover:underline">
                  <span className="truncate font-medium">{l.title}</span>
                </Link>
                <div className="flex shrink-0 items-center gap-2 text-sm">
                  <span className="font-semibold">{money(l.price)}</span>
                  {l.status === "sold" ? (
                    <Badge className="bg-emerald-600">Sold</Badge>
                  ) : pendingFor(l.id) > 0 ? (
                    <Badge className="bg-amber-500">
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
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
            No offers out.{" "}
            <Link href="/" className="underline">
              Browse the market
            </Link>
            .
          </p>
        ) : (
          <ul className="divide-y rounded-lg border">
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
                  <span className="font-semibold">{money(o.amount)}</span>
                  <span
                    className={`rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase ${statusBadge[o.status]}`}
                  >
                    {o.status}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-3">
        <h2 className="flex items-center gap-2 font-semibold">
          <Heart className="size-4 text-rose-500" />
          Watching ({watchlist.length})
        </h2>
        {watchlist.length === 0 ? (
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
            Nothing saved yet. Tap the{" "}
            <Heart className="inline size-3.5 align-text-bottom" /> on any
            listing to watch it — your watchlist is private and syncs live.
          </p>
        ) : (
          <ul className="divide-y rounded-lg border">
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
        title="Sign in to see your market"
        blurb="Your listings, offers received, and bids you've sent — all live. The demo account is prefilled; just hit Log in."
      >
        <Dashboard />
      </AuthGate>
    </MarketProvider>
  );
}
