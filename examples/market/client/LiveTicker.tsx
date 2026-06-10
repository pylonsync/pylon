"use client";

import React from "react";
import { Link, db } from "@pylonsync/react";
import { MarketProvider } from "./MarketProvider";
import { timeAgo, type Listing } from "./market";

// Realtime "just listed" strip. A single live query on Listing; the moment
// anyone (another tab, another visitor) creates a listing, it slides in here
// — no polling, no refetch. This is the part SSR can't do; it's why the
// marketplace feels alive.
function Ticker() {
  const { data } = db.useQuery<Listing>("Listing", {
    where: { status: "active" },
    orderBy: { createdAt: "desc" },
    limit: 6,
  });
  const listings = data ?? [];
  if (listings.length === 0) return null;
  return (
    <div className="flex items-center gap-3 overflow-x-auto rounded-lg border bg-card px-3 py-2 text-sm">
      <span className="flex shrink-0 items-center gap-1.5 font-medium text-emerald-600">
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75" />
          <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
        </span>
        Just listed
      </span>
      <div className="flex items-center gap-4">
        {listings.map((l) => (
          <Link
            key={l.id}
            href={`/listing/${l.id}`}
            className="flex shrink-0 items-center gap-2 whitespace-nowrap hover:underline"
          >
            <span className="font-medium">{l.title}</span>
            <span className="text-muted-foreground">
              {timeAgo(l.createdAt)}
            </span>
          </Link>
        ))}
      </div>
    </div>
  );
}

export function LiveTicker() {
  return (
    <MarketProvider fallback={null}>
      <Ticker />
    </MarketProvider>
  );
}
