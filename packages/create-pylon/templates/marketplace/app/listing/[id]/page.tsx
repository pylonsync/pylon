import React, { Suspense, use } from "react";
import {
  Link,
  useRouteData,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
  type SsrResponse,
} from "@pylonsync/react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from "@/components/ui/empty";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { OfferPanel } from "../../../client/OfferPanel";
import { CategoryIcon } from "../../_components/CategoryIcon";
import { WatchButton } from "../../../client/WatchButton";
import { LiveListingStatus } from "../../../client/LiveListingStatus";
import {
  gradient,
  money,
  conditionLabel,
  type Listing,
  type Offer,
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
  // Listing cards seed this route with the row they already rendered.
  // useRouteData paints that real content immediately, resets scroll at the
  // start of navigation, then upgrades it to the authoritative server row in
  // place. Direct loads still suspend for SSR as normal.
  const listing = useRouteData<Listing | null>(
    () => resolveListing(serverData, id),
    [serverData, id],
  );

  if (!listing) {
    response.setStatus(404);
    return (
      <Empty className="mx-auto min-h-[60vh] max-w-xl border-0">
        <EmptyHeader>
          <p className="text-sm font-medium text-muted-foreground">Unavailable</p>
          <EmptyTitle className="text-3xl tracking-[-0.03em]">
            This listing is no longer available
          </EmptyTitle>
          <EmptyDescription className="max-w-md">
            It may have sold or been removed by the seller. Browse the latest
            finds to discover something similar.
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button asChild size="lg">
            <Link href="/">Browse latest finds</Link>
          </Button>
        </EmptyContent>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col gap-6 pb-10">
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
              width="1200"
              height="1500"
              fetchPriority="high"
              decoding="async"
              className="h-full w-full object-cover outline outline-1 -outline-offset-1 outline-border"
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
          <LiveListingStatus
            listingId={listing.id}
            initialStatus={listing.status}
            mode="overlay"
          />
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

          <Separator className="my-6" />

          <section>
            <h2 className="text-sm font-medium">About this item</h2>
            <p className="mt-2 whitespace-pre-wrap text-pretty text-sm leading-6 text-muted-foreground">
              {listing.description || "The seller has not added a description yet."}
            </p>
          </section>

          <Card className="my-6 grid grid-cols-2 gap-px overflow-hidden rounded-xl bg-border">
            <div className="bg-card p-4">
              <p className="text-xs text-muted-foreground">Condition</p>
              <p className="mt-1 text-sm font-medium">{conditionLabel(listing.condition)}</p>
            </div>
            <div className="bg-card p-4">
              <p className="text-xs text-muted-foreground">Offer status</p>
              <p className="mt-1 text-sm font-medium">
                <LiveListingStatus
                  listingId={listing.id}
                  initialStatus={listing.status}
                />
              </p>
            </div>
          </Card>

          <div className="mt-6">
            <Suspense fallback={<ListingOffers listing={listing} />}>
              <LoadedListingOffers serverData={serverData} listing={listing} />
            </Suspense>
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

function ListingOffers({
  listing,
  initialOffers,
}: {
  listing: Listing;
  initialOffers?: Offer[];
}) {
  return (
    <OfferPanel
      listingId={listing.id}
      sellerId={listing.sellerId}
      sellerName={listing.sellerName}
      title={listing.title}
      price={listing.price}
      status={listing.status}
      initialOffers={initialOffers}
    />
  );
}

function LoadedListingOffers({
  serverData,
  listing,
}: {
  serverData: ServerData;
  listing: Listing;
}) {
  const initialOffers = use(
    serverData.query<Offer>("Offer", { listingId: listing.id }),
  );
  return <ListingOffers listing={listing} initialOffers={initialOffers} />;
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
          <Skeleton className="aspect-[4/5] rounded-[24px]" />
          <div className="flex flex-col gap-3">
            <Skeleton className="h-6 w-2/3" />
            <Skeleton className="h-8 w-1/3" />
            <Skeleton className="h-24" />
          </div>
        </div>
      }
    >
      <Detail
        key={params.id}
        serverData={serverData}
        response={response}
        id={params.id}
      />
    </Suspense>
  );
}
