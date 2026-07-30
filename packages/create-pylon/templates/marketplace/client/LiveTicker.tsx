"use client";

import React from "react";
import { Link, db } from "@pylonsync/react";
import { Tag, BadgeCheck } from "lucide-react";
import { MarketProvider } from "./MarketProvider";
import { money, timeAgo, type Listing, type Offer } from "./market";

// Realtime activity strip. Two live queries — new listings AND completed sales
// (accepted offers) — merged into one feed. The moment anyone lists an item or
// a sale closes (in another tab, by another visitor), it slides in here. No
// polling, no refetch. This is the part SSR can't do; it's why the marketplace
// feels alive.
type Activity = {
  key: string;
  kind: "listed" | "sold";
  title: string;
  href: string;
  amount?: number;
  at: string;
};

function Ticker() {
  const { data: listingData } = db.useQuery<Listing>("Listing", {
    where: { status: "active" },
    orderBy: { createdAt: "desc" },
    limit: 8,
  });
  const { data: saleData } = db.useQuery<Offer>("Offer", {
    where: { status: "accepted" },
    orderBy: { createdAt: "desc" },
    limit: 8,
  });

  const events: Activity[] = [
    ...(listingData ?? []).map((l) => ({
      key: `l-${l.id}`,
      kind: "listed" as const,
      title: l.title,
      href: `/listing/${l.slug || l.id}`,
      at: l.createdAt,
    })),
    ...(saleData ?? []).map((o) => ({
      key: `s-${o.id}`,
      kind: "sold" as const,
      title: o.listingTitle,
      href: `/listing/${o.listingId}`,
      amount: o.amount,
      at: o.createdAt,
    })),
  ]
    .sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
    .slice(0, 10);

  if (events.length === 0) return null;

  return (
    <div className="flex min-h-11 items-center gap-3 overflow-hidden rounded-2xl bg-card px-4 py-2 text-sm shadow-[var(--shadow-border)]">
      <span className="flex shrink-0 items-center gap-1.5 font-medium text-emerald-600">
        <span className="relative flex size-2">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-400 opacity-75" />
          <span className="relative inline-flex size-2 rounded-full bg-emerald-500" />
        </span>
        Live
      </span>
      {/* Auto-scrolling marquee — no scrollbar. Items are doubled so the loop
          (translateX -50%) is seamless; hover pauses so you can click. The
          edge mask fades items in/out for a clean "live feed" look. */}
      <div className="relative flex-1 overflow-hidden [mask-image:linear-gradient(to_right,transparent,#000_1.5rem,#000_calc(100%-1.5rem),transparent)]">
        <div className="market-marquee flex w-max items-center gap-6">
          {[...events, ...events].map((e, i) => (
            <Link
              key={`${e.key}-${i}`}
              href={e.href}
              className="flex shrink-0 items-center gap-1.5 whitespace-nowrap hover:underline"
              aria-hidden={i >= events.length}
              tabIndex={i >= events.length ? -1 : undefined}
            >
              {e.kind === "sold" ? (
                <BadgeCheck className="size-3.5 text-emerald-600" />
              ) : (
                <Tag className="size-3.5 text-muted-foreground" />
              )}
              <span className="font-medium">{e.title}</span>
              <span className="text-muted-foreground">
                {e.kind === "sold" && e.amount != null
                  ? `sold · ${money(e.amount)}`
                  : timeAgo(e.at)}
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
