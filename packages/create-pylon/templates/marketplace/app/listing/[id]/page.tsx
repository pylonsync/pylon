import React, { Suspense, use } from "react";
import {
  Link,
  type GenerateMetadata,
  type Metadata,
  type PageProps,
  type ServerData,
  type SsrResponse,
} from "@pylonsync/react";
import { Badge } from "../../../ui/badge";
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
  if (!l) return { title: "Listing not found · Pylon Market" };
  return {
    title: `${l.title} — ${money(l.price)} · Pylon Market`,
    description:
      l.description?.slice(0, 155) ||
      `${l.title} for sale on Pylon Market (${conditionLabel(l.condition)}).`,
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
    <div className="space-y-6">
      <Link
        href="/"
        className="text-sm text-muted-foreground hover:text-foreground"
      >
        ← Back to the market
      </Link>

      <div className="grid gap-8 md:grid-cols-2">
        <div
          className="relative flex aspect-square items-center justify-center rounded-2xl text-white/90"
          style={{ background: gradient(listing.seed || listing.id) }}
        >
          <CategoryIcon category={listing.category} className="size-28" />
          <WatchButton
            listingId={listing.id}
            listingTitle={listing.title}
            className="absolute right-3 top-3"
          />
          {listing.status === "sold" ? (
            <span className="absolute inset-0 grid place-items-center rounded-2xl bg-black/55 text-2xl font-bold uppercase tracking-wide">
              Sold
            </span>
          ) : null}
        </div>

        <div className="flex flex-col gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-xs uppercase tracking-wider text-muted-foreground">
              <span>{listing.category}</span>
              <span>·</span>
              <Badge variant="outline" className="text-[10px]">
                {conditionLabel(listing.condition)}
              </Badge>
            </div>
            <h1 className="text-2xl font-semibold tracking-tight">
              {listing.title}
            </h1>
            <p className="text-3xl font-semibold tabular-nums">{money(listing.price)}</p>
            <p className="text-sm text-muted-foreground">
              Listed by <span className="font-medium">{listing.sellerName}</span>
            </p>
          </div>

          {listing.description ? (
            <p className="whitespace-pre-wrap text-sm leading-relaxed text-foreground/80">
              {listing.description}
            </p>
          ) : null}

          {/* Realtime client island: live offers, accept/decline. */}
          <div className="mt-2">
            <OfferPanel
              listingId={listing.id}
              sellerId={listing.sellerId}
              sellerName={listing.sellerName}
              title={listing.title}
              price={listing.price}
              status={listing.status}
            />
          </div>
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
          <div className="aspect-square animate-pulse rounded-2xl bg-muted" />
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
