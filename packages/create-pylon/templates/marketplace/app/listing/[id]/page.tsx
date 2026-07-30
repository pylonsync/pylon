import React, { Suspense, use } from "react";
import {
  Link,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
  type SsrResponse,
} from "@pylonsync/react";
import { OfferPanel } from "../../../client/OfferPanel";
import { CategoryIcon } from "../../_components/CategoryIcon";
import { WatchButton } from "../../../client/WatchButton";
import {
  gradient,
  money,
  conditionLabel,
  type Listing,
} from "../../../client/market";

// Resolve a listing from the URL segment, which is its slug
// ("herman-miller-aeron-a1f3"). Falls back to a raw id lookup so older
// id-shaped links keep working.
async function resolveListing(
  serverData: ServerData,
  key: string,
): Promise<Listing | null> {
  return (
    (await serverData.lookup<Listing>("Listing", "slug", key)) ??
    (await serverData.get<Listing>("Listing", key))
  );
}

// The listing page is anonymous + public (the watch button + offer panel are
// client islands with their own auth), so its SSR output is shared across
// visitors — an ISR candidate. `revalidate` serves it from cache, emits
// stale-while-revalidate, and makes <Link>'s prefetch reusable so a click hits
// cache instead of a live render. Short TTL because a listing's sold status is
// time-sensitive; the realtime offer panel keeps the live bits fresh regardless.
export const revalidate = 60;

// Data-driven SEO: the title + description come from the listing itself,
// fetched on the server. `generateMetadata` is handed the same PageProps as
// the page (params + serverData), so it reads the row directly.
export const generateMetadata: GenerateMetadata = async ({
  params,
  serverData,
}): Promise<Metadata> => {
  const l = await resolveListing(serverData, params.id);
  if (!l) return { title: "Listing not found | Reprise" };
  return {
    title: `${l.title} | ${money(l.price)} | Reprise`,
    description:
      l.description?.slice(0, 155) ||
      `${l.title} for sale on Reprise (${conditionLabel(l.condition)}).`,
  };
};

function Detail({
  serverData,
  response,
  id,
}: {
  serverData: ServerData;
  response: SsrResponse;
  id: string;
}) {
  const listing = use(resolveListing(serverData, id));

  if (!listing) {
    response.setStatus(404);
    return (
      <div className="rounded-xl border border-dashed p-12 text-center">
        <p className="font-medium">This listing is gone.</p>
        <Link href="/" className="mt-2 inline-block text-sm underline">
          Back to the market
        </Link>
      </div>
    );
  }

  return (
    <div className="space-y-6 pb-10">
      <Link
        href="/"
        className="inline-flex min-h-10 items-center text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        ← Back to browse
      </Link>

      <div className="grid gap-8 lg:grid-cols-[minmax(0,1.16fr)_minmax(360px,.84fr)] lg:gap-12">
        <div className="relative aspect-[4/5] overflow-hidden rounded-[24px] bg-muted shadow-[var(--shadow-border)] sm:aspect-[6/5] lg:aspect-[4/5]">
          {listing.imageUrl ? (
            <img
              src={listing.imageUrl}
              alt={listing.title}
              fetchPriority="high"
              decoding="async"
              className="h-full w-full object-cover outline outline-1 -outline-offset-1 outline-black/10 dark:outline-white/10"
            />
          ) : (
            <div
              className="flex h-full w-full items-center justify-center text-white/90"
              style={{ background: gradient(listing.seed || listing.id) }}
            >
              <CategoryIcon category={listing.category} className="size-28" />
            </div>
          )}
          <WatchButton
            listingId={listing.id}
            listingTitle={listing.title}
            className="absolute right-4 top-4"
          />
          {listing.status === "sold" ? (
            <span className="absolute inset-0 grid place-items-center bg-black/55 text-2xl font-semibold text-white">
              Sold
            </span>
          ) : null}
        </div>

        <div className="flex min-w-0 flex-col">
          <div>
            <p className="text-sm capitalize text-muted-foreground">
              {listing.category} / {conditionLabel(listing.condition)}
            </p>
            <h1 className="mt-2 text-balance text-3xl font-semibold leading-tight tracking-[-0.035em] sm:text-4xl">
              {listing.title}
            </h1>
            <p className="mt-3 text-4xl font-semibold tracking-[-0.03em] tabular-nums">
              {money(listing.price)}
            </p>
            <p className="mt-2 text-sm text-muted-foreground">
              Listed by{" "}
              <span className="font-medium text-foreground">{listing.sellerName}</span>
            </p>
          </div>

          <div className="my-6 h-px bg-border" />

          <section>
            <h2 className="text-sm font-medium">About this item</h2>
            <p className="mt-2 whitespace-pre-wrap text-pretty text-sm leading-6 text-muted-foreground">
              {listing.description || "The seller has not added a description yet."}
            </p>
          </section>

          <div className="my-6 grid grid-cols-2 gap-px overflow-hidden rounded-xl bg-border shadow-[var(--shadow-border)]">
            <div className="bg-card p-4">
              <p className="text-xs text-muted-foreground">Condition</p>
              <p className="mt-1 text-sm font-medium">{conditionLabel(listing.condition)}</p>
            </div>
            <div className="bg-card p-4">
              <p className="text-xs text-muted-foreground">Offer status</p>
              <p className="mt-1 text-sm font-medium">
                {listing.status === "active" ? "Open to offers" : "Sold"}
              </p>
            </div>
          </div>

          <div className="mt-6">
            <OfferPanel
              listingId={listing.id}
              sellerId={listing.sellerId}
              sellerName={listing.sellerName}
              title={listing.title}
              price={listing.price}
              status={listing.status}
            />
          </div>

          <p className="mt-4 text-pretty text-xs leading-5 text-muted-foreground">
            Reprise keeps offers and listing status in sync. Confirm payment and
            delivery details with the seller before completing a transaction.
          </p>
        </div>
      </div>
    </div>
  );
}

export default function ListingPage({
  params,
  serverData,
  response,
}: PageProps) {
  return (
    <Suspense
      fallback={
        <div className="grid gap-8 md:grid-cols-2">
          <div className="aspect-[4/5] animate-pulse rounded-[24px] bg-muted" />
          <div className="space-y-3">
            <div className="h-6 w-2/3 animate-pulse rounded bg-muted" />
            <div className="h-8 w-1/3 animate-pulse rounded bg-muted" />
            <div className="h-24 animate-pulse rounded bg-muted" />
          </div>
        </div>
      }
    >
      <Detail serverData={serverData} response={response} id={params.id} />
    </Suspense>
  );
}
