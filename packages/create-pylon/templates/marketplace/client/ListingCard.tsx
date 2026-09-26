import React from "react";
import { Link } from "@pylonsync/react";
import { cn } from "@/lib/utils";
import { listingSrcSet } from "../lib/catalog";
import { CategoryIcon } from "../app/_components/CategoryIcon";
import { WatchButton } from "./WatchButton";
import {
  conditionLabel,
  gradient,
  money,
  timeAgo,
  type Listing,
} from "./market";

// One tile in the browse grid. Pure render (no data hooks), so the server
// grid and the live client grid draw the same markup.
export function ListingCard({
  listing,
  pendingOffers = 0,
  fresh = false,
  eager = false,
  sizes = "(min-width: 1280px) 20vw, (min-width: 768px) 30vw, 50vw",
  className,
}: {
  listing: Listing;
  /** Live count of pending offers; 0 hides the pill. */
  pendingOffers?: number;
  /** Arrived over sync after first paint. */
  fresh?: boolean;
  /** Load the photo eagerly (first row). */
  eager?: boolean;
  sizes?: string;
  className?: string;
}) {
  const sold = listing.status === "sold";
  const href = `/listing/${listing.slug || listing.id}`;

  return (
    <article
      className={cn(
        "group relative min-w-0",
        fresh && "market-rise",
        className,
      )}
    >
      <Link href={href} seed={listing} className="block rounded-xl">
        <div className="relative aspect-[4/5] overflow-hidden rounded-xl bg-muted">
          {listing.imageUrl ? (
            <img
              src={listing.imageUrl}
              srcSet={listingSrcSet(listing.imageUrl)}
              sizes={sizes}
              alt={listing.title}
              width="640"
              height="800"
              loading={eager ? "eager" : "lazy"}
              fetchPriority={eager ? "high" : undefined}
              decoding="async"
              className="size-full object-cover transition-transform duration-700 ease-[var(--ease-out-soft)] group-hover:scale-[1.035]"
            />
          ) : (
            <div
              className="flex size-full items-center justify-center text-white/85"
              style={{ background: gradient(listing.seed || listing.id) }}
            >
              <CategoryIcon category={listing.category} className="size-12" />
            </div>
          )}
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 rounded-xl ring-1 ring-inset ring-black/5"
          />

          {sold ? (
            <span className="absolute inset-0 grid place-items-center bg-black/45">
              <span className="rounded-full bg-white px-3.5 py-1 text-xs font-semibold text-neutral-900">
                Sold
              </span>
            </span>
          ) : fresh ? (
            <span className="absolute left-2.5 top-2.5 inline-flex items-center gap-1.5 rounded-full bg-signal px-2.5 py-1 text-[11px] font-semibold text-white shadow-[var(--shadow-float)]">
              Just listed
            </span>
          ) : pendingOffers > 0 ? (
            <span className="absolute left-2.5 top-2.5 inline-flex items-center gap-1.5 rounded-full bg-white/92 px-2.5 py-1 text-[11px] font-medium text-neutral-900 shadow-[var(--shadow-float)] backdrop-blur">
              <span className="size-1.5 rounded-full bg-signal" aria-hidden="true" />
              {pendingOffers} {pendingOffers === 1 ? "offer" : "offers"}
            </span>
          ) : null}
        </div>

        <div className="flex flex-col gap-0.5 px-0.5 pt-2.5">
          <div className="flex items-baseline justify-between gap-2">
            <p className="text-[15px] font-semibold tabular-nums tracking-[-0.01em]">
              {money(listing.price)}
            </p>
            <p className="shrink-0 text-xs text-muted-foreground" suppressHydrationWarning>
              {timeAgo(listing.createdAt)}
            </p>
          </div>
          <h3 className="truncate text-sm leading-5 text-foreground/90">
            {listing.title}
          </h3>
          <p className="truncate text-[13px] leading-5 text-muted-foreground">
            {[listing.location, conditionLabel(listing.condition)]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </div>
      </Link>
      <WatchButton
        listingId={listing.id}
        listingTitle={listing.title}
        className="absolute right-2.5 top-2.5"
      />
    </article>
  );
}
