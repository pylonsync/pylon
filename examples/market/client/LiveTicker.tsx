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
    <div className="flex items-center gap-3 overflow-hidden rounded-lg border bg-card px-3 py-2 text-sm">
      <span className="flex shrink-0 items-center gap-1.5 font-medium text-emerald-600">
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75" />
          <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
        </span>
        Just listed
      </span>
      {/* Auto-scrolling marquee — no scrollbar. Items are doubled so the loop
          (translateX -50%) is seamless; hover pauses so you can click. The
          edge mask fades items in/out for a clean "live feed" look. */}
      <div className="relative flex-1 overflow-hidden [mask-image:linear-gradient(to_right,transparent,#000_1.5rem,#000_calc(100%-1.5rem),transparent)]">
        <div className="market-marquee flex w-max items-center gap-6">
          {[...listings, ...listings].map((l, i) => (
            <Link
              key={`${l.id}-${i}`}
              href={`/listing/${l.id}`}
              className="flex shrink-0 items-center gap-2 whitespace-nowrap hover:underline"
              aria-hidden={i >= listings.length}
              tabIndex={i >= listings.length ? -1 : undefined}
            >
              <span className="font-medium">{l.title}</span>
              <span className="text-muted-foreground">
                {timeAgo(l.createdAt)}
              </span>
            </Link>
          ))}
        </div>
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
