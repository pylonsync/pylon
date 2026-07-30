"use client";

import React from "react";
import { Link, db } from "@pylonsync/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Spinner } from "@/components/ui/spinner";
import { AuthGate, MarketProvider, useIdentity } from "./MarketProvider";
import { Heart } from "lucide-react";
import { money, timeAgo, type Listing, type Offer, type Watch } from "./market";

type BadgeVariant = "default" | "secondary" | "destructive" | "outline" | "success" | "warning";

const statusVariant: Record<string, BadgeVariant> = {
  pending: "warning",
  accepted: "success",
  declined: "outline",
};

function uniqueById<T extends { id: string }>(rows: T[]): T[] {
  return Array.from(new Map(rows.map((row) => [row.id, row])).values());
}

function Dashboard() {
  // Rendered inside <AuthGate>, so identity is non-null here.
  const identity = useIdentity();
  const userId = identity?.userId ?? "";
  const name = identity?.name ?? "you";

  // Three live queries. Listings + offers are public-read, then narrowed to
  // the signed-in user in memory; Watch remains policy-scoped to its owner.
  // Everything updates in place: a new offer on my listing, a seller
  // answering my bid — no refresh.
  const { data: listings } = db.useQuery<Listing>("Listing", {
    orderBy: { createdAt: "desc" },
  });
  const { data: offers } = db.useQuery<Offer>("Offer", {
    orderBy: { createdAt: "desc" },
  });
  const { data: watching } = db.useQuery<Watch>("Watch", {
    where: { userId },
    orderBy: { createdAt: "desc" },
  });

  const allListings = uniqueById(listings ?? []);
  const listingById = new Map(
    allListings.map((listing) => [listing.id, listing]),
  );
  const myListings = allListings.filter(
    (listing) => listing.sellerId === userId,
  );
  const allOffers = uniqueById(offers ?? []);
  const myOffers = allOffers.filter((offer) => offer.buyerId === userId);
  const watchlist = uniqueById(watching ?? []);
  const inbound = allOffers.filter((offer) => offer.sellerId === userId);
  const pendingFor = (listingId: string) =>
    inbound.filter((o) => o.listingId === listingId && o.status === "pending")
      .length;

  return (
    <div className="flex flex-col gap-10 py-2 sm:py-6">
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

      <section className="flex flex-col gap-3">
        <h2 className="font-semibold">Your listings ({myListings.length})</h2>
        {myListings.length === 0 ? (
          <Empty className="border-0 bg-card py-8 shadow-[var(--shadow-border)]">
            <EmptyHeader>
              <EmptyTitle className="text-base">Nothing listed yet</EmptyTitle>
              <EmptyDescription>
                Post your first item and it will appear here.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button asChild variant="outline">
                <Link href="/sell">Post your first item</Link>
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <Card>
          <ul className="divide-y overflow-hidden">
            {myListings.map((l) => (
              <li key={l.id} className="flex items-center justify-between gap-3 p-3">
                <Link
                  href={`/listing/${l.slug || l.id}`}
                  seed={l}
                  className="flex min-w-0 items-center gap-3 hover:underline"
                >
                  {l.imageUrl ? (
                    <img
                      src={l.imageUrl}
                      alt=""
                      width="48"
                      height="48"
                      loading="lazy"
                      className="size-12 shrink-0 rounded-lg object-cover outline outline-1 -outline-offset-1 outline-border"
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
          </Card>
        )}
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="font-semibold">Offers you've made ({myOffers.length})</h2>
        {myOffers.length === 0 ? (
          <Empty className="border-0 bg-card py-8 shadow-[var(--shadow-border)]">
            <EmptyHeader>
              <EmptyTitle className="text-base">No offers out</EmptyTitle>
              <EmptyDescription>
                When you make an offer, its status will stay synced here.
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent>
              <Button asChild variant="outline">
                <Link href="/">Browse the market</Link>
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <Card>
          <ul className="divide-y overflow-hidden">
            {myOffers.map((o) => (
              <li
                key={o.id}
                className="flex flex-col items-start gap-2 p-3 sm:flex-row sm:items-center sm:justify-between"
              >
                <Link
                  href={`/listing/${o.listingId}`}
                  seed={listingById.get(o.listingId)}
                  className="min-w-0 max-w-full hover:underline"
                >
                  <span className="block truncate font-medium">{o.listingTitle}</span>
                  <span className="block text-sm text-muted-foreground sm:mt-0 sm:inline sm:pl-2">
                    {timeAgo(o.createdAt)}
                  </span>
                </Link>
                <div className="flex shrink-0 self-end items-center gap-2 text-sm sm:self-auto">
                  <span className="font-semibold tabular-nums">{money(o.amount)}</span>
                  <Badge
                    variant={statusVariant[o.status] ?? "outline"}
                    className="capitalize"
                  >
                    {o.status}
                  </Badge>
                </div>
              </li>
            ))}
          </ul>
          </Card>
        )}
      </section>

      <section className="flex flex-col gap-3">
        <h2 className="flex items-center gap-2 font-semibold">
          <Heart
            aria-hidden="true"
            className="size-4 fill-foreground text-foreground"
          />
          Watching ({watchlist.length})
        </h2>
        {watchlist.length === 0 ? (
          <Empty className="border-0 bg-card py-8 shadow-[var(--shadow-border)]">
            <EmptyHeader>
              <EmptyTitle className="text-base">Nothing saved yet</EmptyTitle>
              <EmptyDescription>
                Use the heart on any listing to save it privately here.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          <Card>
          <ul className="divide-y overflow-hidden">
            {watchlist.map((w) => (
              <li key={w.id} className="flex items-center justify-between gap-3 p-3">
                <Link
                  href={`/listing/${w.listingId}`}
                  seed={listingById.get(w.listingId)}
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
          </Card>
        )}
      </section>
    </div>
  );
}

export function MyMarket() {
  return (
    <MarketProvider
      fallback={
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Spinner />
          Loading your market…
        </div>
      }
    >
      <AuthGate
        title="Sign in to open your dashboard"
        blurb="Your listings, saved finds, and offers stay synced here. The demo account is ready; just select Log in."
        headingLevel={1}
      >
        <Dashboard />
      </AuthGate>
    </MarketProvider>
  );
}
