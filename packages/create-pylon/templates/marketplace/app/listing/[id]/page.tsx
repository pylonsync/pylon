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
import { ChevronRight, MapPin } from "lucide-react";
import { OfferPanel } from "../../../client/OfferPanel";
import { CategoryIcon } from "../../_components/CategoryIcon";
import { WatchButton } from "../../../client/WatchButton";
import { LiveListingStatus } from "../../../client/LiveListingStatus";
import { ListingCard } from "../../../client/ListingCard";
import {
  avatarColor,
  conditionLabel,
  gradient,
  initials,
  money,
  timeAgo,
  type Listing,
  type Offer,
} from "../../../client/market";
import { categoryLabel, listingSrcSet } from "../../../lib/catalog";

// Resolve a listing from the URL segment, which is its slug
// ("danish-teak-lounge-chair-oatmeal-wool-a1f3"). Falls back to a raw id
// lookup so id-shaped links keep working.
async function resolveListing(
  serverData: ServerData,
  key: string,
): Promise<Listing | null> {
  return (
    (await serverData.lookup<Listing>("Listing", "slug", key)) ??
    (await serverData.get<Listing>("Listing", key))
  );
}

// The listing page is anonymous + public (the watch button and offer panel
// are client islands with their own auth), so its SSR output is shared across
// visitors. `revalidate` serves it from cache with stale-while-revalidate.
// The TTL is short because sold status is time-sensitive; the live islands
// keep offers and status current regardless.
export const revalidate = 60;

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
  // useRouteData paints that row immediately, then upgrades it to the
  // server row in place. Direct loads suspend for SSR as normal.
  const listing = useRouteData<Listing | null>(
    () => resolveListing(serverData, id),
    [serverData, id],
  );

  if (!listing) {
    response.setStatus(404);
    return (
      <div className="mx-auto flex min-h-[60vh] max-w-xl flex-col items-center justify-center text-center">
        <h1 className="font-display text-5xl">This listing is gone</h1>
        <p className="mt-3 text-muted-foreground">
          The seller removed it, or the link is wrong.
        </p>
        <Link
          href="/"
          className="mt-6 inline-flex min-h-11 items-center rounded-full bg-primary px-5 text-sm font-medium text-primary-foreground"
        >
          Browse listings
        </Link>
      </div>
    );
  }

  const facts: Array<[string, string]> = [
    ["Condition", conditionLabel(listing.condition)],
    ["Category", categoryLabel(listing.category)],
    ["Location", listing.location || "Not given"],
    ["Listed", timeAgo(listing.createdAt)],
  ];

  return (
    <div className="pb-6">
      <nav aria-label="Breadcrumb" className="py-4 text-[13px] text-muted-foreground">
        <ol className="flex min-w-0 items-center gap-1.5 whitespace-nowrap">
          <li>
            <Link href="/" className="hover:text-foreground">
              All listings
            </Link>
          </li>
          <ChevronRight aria-hidden="true" className="size-3.5" />
          <li>
            <a
              href={`/?category=${listing.category}`}
              className="hover:text-foreground"
            >
              {categoryLabel(listing.category)}
            </a>
          </li>
          <ChevronRight aria-hidden="true" className="size-3.5" />
          <li className="min-w-0 truncate text-foreground" aria-current="page">
            {listing.title}
          </li>
        </ol>
      </nav>

      <div className="grid gap-8 lg:grid-cols-[minmax(0,560px)_minmax(0,1fr)] lg:gap-12 xl:gap-16">
        <div className="lg:sticky lg:top-20 lg:self-start">
          <div className="relative aspect-[4/5] overflow-hidden rounded-2xl bg-muted">
            {listing.imageUrl ? (
              <img
                src={listing.imageUrl}
                srcSet={listingSrcSet(listing.imageUrl)}
                sizes="(min-width: 1024px) 560px, 100vw"
                alt={listing.title}
                width="1120"
                height="1400"
                fetchPriority="high"
                decoding="async"
                className="size-full object-cover"
              />
            ) : (
              <div
                className="flex size-full items-center justify-center text-white/85"
                style={{ background: gradient(listing.seed || listing.id) }}
              >
                <CategoryIcon category={listing.category} className="size-24" />
              </div>
            )}
            <span
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 rounded-2xl ring-1 ring-inset ring-black/5"
            />
            <WatchButton
              listingId={listing.id}
              listingTitle={listing.title}
              className="absolute right-4 top-4 size-10"
            />
            <LiveListingStatus
              listingId={listing.id}
              initialStatus={listing.status}
              mode="overlay"
            />
          </div>
        </div>

        <div className="flex min-w-0 max-w-[660px] flex-col">
          <h1 className="font-display text-[38px] leading-[1.04] tracking-[-0.01em] sm:text-[46px]">
            {listing.title}
          </h1>
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <p className="text-[30px] font-semibold leading-none tabular-nums tracking-[-0.02em]">
              {money(listing.price)}
            </p>
            <LiveListingStatus
              listingId={listing.id}
              initialStatus={listing.status}
              mode="pill"
            />
          </div>
          <p className="mt-3 flex flex-wrap items-center gap-x-2 text-sm text-muted-foreground">
            <span>{conditionLabel(listing.condition)}</span>
            <span aria-hidden="true">·</span>
            {listing.location ? (
              <>
                <span className="inline-flex items-center gap-1">
                  <MapPin aria-hidden="true" className="size-3.5" strokeWidth={1.75} />
                  {listing.location}
                </span>
                <span aria-hidden="true">·</span>
              </>
            ) : null}
            <span suppressHydrationWarning>Listed {timeAgo(listing.createdAt)}</span>
          </p>

          <div className="mt-6 flex items-center gap-3 rounded-2xl bg-card p-3 shadow-[var(--shadow-border)]">
            <span
              aria-hidden="true"
              className="grid size-11 shrink-0 place-items-center rounded-full text-sm font-semibold text-white"
              style={{ background: avatarColor(listing.sellerName) }}
            >
              {initials(listing.sellerName)}
            </span>
            <div className="min-w-0">
              <p className="truncate text-sm font-medium">{listing.sellerName}</p>
              <p className="truncate text-[13px] text-muted-foreground">
                Seller{listing.location ? ` in ${listing.location}` : ""}
              </p>
            </div>
          </div>

          <div className="mt-6">
            <Suspense fallback={<ListingOffers listing={listing} />}>
              <LoadedListingOffers serverData={serverData} listing={listing} />
            </Suspense>
          </div>

          <section className="mt-10">
            <h2 className="text-base font-semibold">Description</h2>
            <p className="mt-2 whitespace-pre-wrap text-[15px] leading-7 text-foreground/80">
              {listing.description || "The seller has not added a description."}
            </p>
          </section>

          <dl className="mt-6 grid grid-cols-2 gap-px overflow-hidden rounded-2xl bg-border shadow-[var(--shadow-border)]">
            {facts.map(([label, value]) => (
              <div key={label} className="bg-card px-4 py-3">
                <dt className="text-xs text-muted-foreground">{label}</dt>
                <dd className="mt-0.5 text-sm font-medium" suppressHydrationWarning>
                  {value}
                </dd>
              </div>
            ))}
          </dl>

          <p className="mt-6 text-xs leading-5 text-muted-foreground">
            Offers and sold status sync live. Arrange payment and pickup
            directly with the seller.
          </p>
        </div>
      </div>

      <Suspense fallback={null}>
        <MoreListings serverData={serverData} listing={listing} />
      </Suspense>
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

// Four more active listings: same category first, then the newest others.
function MoreListings({
  serverData,
  listing,
}: {
  serverData: ServerData;
  listing: Listing;
}) {
  const active = use(
    serverData.query<Listing>("Listing", { status: "active" }),
  );
  const others = active
    .filter((l) => l.id !== listing.id)
    .sort((a, b) => {
      const sameA = a.category === listing.category ? 0 : 1;
      const sameB = b.category === listing.category ? 0 : 1;
      return sameA - sameB || b.createdAt.localeCompare(a.createdAt);
    })
    .slice(0, 5);
  if (others.length === 0) return null;

  return (
    <section className="mt-16 border-t border-border/70 pt-8">
      <div className="flex items-end justify-between gap-4">
        <h2 className="font-display text-3xl">More to look at</h2>
        <Link
          href="/"
          className="text-sm font-medium text-muted-foreground hover:text-foreground"
        >
          All listings
        </Link>
      </div>
      <div className="mt-5 grid grid-cols-2 gap-x-3 gap-y-6 sm:gap-x-4 md:grid-cols-3 lg:grid-cols-5">
        {others.map((l, index) => (
          <ListingCard
            key={l.id}
            listing={l}
            sizes="(min-width: 1024px) 18vw, 50vw"
            className={index === 4 ? "hidden lg:block" : undefined}
          />
        ))}
      </div>
    </section>
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
        <div className="grid gap-10 pt-14 lg:grid-cols-[580px_1fr]">
          <div className="aspect-[4/5] rounded-2xl bg-muted" />
          <div className="flex flex-col gap-4">
            <div className="h-12 w-3/4 rounded-lg bg-muted" />
            <div className="h-8 w-1/4 rounded-lg bg-muted" />
            <div className="h-40 rounded-2xl bg-muted" />
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
