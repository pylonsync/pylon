"use client";

import React, { useRef } from "react";
import { db } from "@pylonsync/react";
import { browseListings, pendingOfferCounts } from "../lib/catalog";
import { ListingCard } from "./ListingCard";
import { MarketProvider } from "./MarketProvider";
import type { Listing, Offer } from "./market";

interface GridProps {
  /** Active listings the server rendered. */
  initial: Listing[];
  /** Pending offers the server rendered, for the offer-count pills. */
  initialOffers: Offer[];
  category: string;
  query: string;
  sort: string;
}

const GRID =
  "grid grid-cols-2 gap-x-3 gap-y-6 sm:gap-x-4 sm:gap-y-8 md:grid-cols-3 xl:grid-cols-4";

// The browse grid. The server renders it from `initial`; once the sync
// engine connects, the same grid switches to two live queries:
//   - Listing: a listing posted in any tab rises into place with a
//     "Just listed" pill, and one that sells gets a Sold overlay.
//   - Offer: the "N offers" pill on each card follows pending offers.
export function LiveGrid(props: GridProps) {
  return (
    <MarketProvider fallback={<StaticGrid {...props} />}>
      <Live {...props} />
    </MarketProvider>
  );
}

function StaticGrid({ initial, initialOffers, category, query, sort }: GridProps) {
  const rows = browseListings(initial, { category, query, sort });
  const counts = pendingOfferCounts(initialOffers);
  return (
    <div className={GRID}>
      {rows.map((listing, index) => (
        <ListingCard
          key={listing.id}
          listing={listing}
          pendingOffers={counts.get(listing.id) ?? 0}
          eager={index < 4}
        />
      ))}
    </div>
  );
}

function Live({ initial, initialOffers, category, query, sort }: GridProps) {
  // Ids present at first paint. Anything else arrived live.
  const firstIds = useRef<Set<string>>(new Set(initial.map((l) => l.id)));

  const { data: liveListings } = db.useQuery<Listing>("Listing", {
    orderBy: { createdAt: "desc" },
  });
  const { data: liveOffers, loading: offersLoading } = db.useQuery<Offer>(
    "Offer",
    { where: { status: "pending" } },
  );

  const byId = new Map(initial.map((l) => [l.id, l]));
  for (const listing of liveListings ?? []) byId.set(listing.id, listing);
  // Keep a listing that sells while on screen (it shows the Sold overlay);
  // skip listings that were already sold before this page loaded.
  const rows = browseListings(
    [...byId.values()].filter(
      (l) => l.status === "active" || firstIds.current.has(l.id),
    ),
    { category, query, sort },
  );
  const counts = pendingOfferCounts(
    offersLoading ? initialOffers : (liveOffers ?? []),
  );

  return (
    <div className={GRID}>
      {rows.map((listing, index) => (
        <ListingCard
          key={listing.id}
          listing={listing}
          pendingOffers={counts.get(listing.id) ?? 0}
          fresh={!firstIds.current.has(listing.id)}
          eager={index < 4}
        />
      ))}
    </div>
  );
}
