import React, { Suspense, use } from "react";
import {
  Link,
  type Metadata,
  type PageProps,
  type ServerData,
} from "@pylonsync/react";
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
import {
  Field,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  NativeSelect,
  NativeSelectOption,
} from "@/components/ui/native-select";
import { Skeleton } from "@/components/ui/skeleton";
import { LiveTicker } from "../client/LiveTicker";
import { SeedOnEmpty } from "../client/SeedOnEmpty";
import { CategoryIcon } from "./_components/CategoryIcon";
import { WatchButton } from "../client/WatchButton";
import { ScrollToListingsLink } from "../client/ScrollToListingsLink";
import { gradient, money, conditionLabel, type Listing } from "../client/market";
import { browseListings } from "../lib/catalog";

export const metadata: Metadata = {
  title: "Reprise | Distinctive secondhand finds",
  description:
    "Browse distinctive secondhand furniture, technology, fashion, and more. Save favorites, make offers, and follow every update live.",
};

const CATEGORIES = [
  "all",
  "furniture",
  "electronics",
  "cameras",
  "bikes",
  "audio",
  "kitchen",
  "instruments",
  "outdoor",
  "apparel",
];

function browseHref(category: string, query: string, sort: string): string {
  const params = new URLSearchParams();
  if (category !== "all") params.set("category", category);
  if (query) params.set("q", query);
  if (sort !== "latest") params.set("sort", sort);
  const suffix = params.toString();
  return suffix ? `/?${suffix}#listings` : "/#listings";
}

function ListingImage({ listing }: { listing: Listing }) {
  if (listing.imageUrl) {
    return (
      <img
        src={listing.imageUrl}
        alt={listing.title}
        width="600"
        height="750"
        loading="lazy"
        decoding="async"
        className="h-full w-full object-cover outline outline-1 -outline-offset-1 outline-border"
      />
    );
  }

  return (
    <div
      className="flex h-full w-full items-center justify-center text-white/90"
      style={{ background: gradient(listing.seed || listing.id) }}
    >
      <CategoryIcon category={listing.category} className="size-14" />
    </div>
  );
}

function Grid({
  serverData,
  category,
  query,
  sort,
}: {
  serverData: ServerData;
  category: string;
  query: string;
  sort: string;
}) {
  const active = use(serverData.query<Listing>("Listing", { status: "active" }));
  const listings = browseListings(active, { category, query, sort });

  if (listings.length === 0) {
    return (
      <>
        <Empty className="border-0 bg-card py-16 shadow-[var(--shadow-border)]">
          <EmptyHeader>
            <EmptyTitle>
              {active.length === 0
                ? "Adding sample finds…"
                : "No matching finds yet"}
            </EmptyTitle>
            <EmptyDescription>
              {active.length === 0
                ? "The catalog will appear in a moment."
                : "Try another category or a broader search."}
            </EmptyDescription>
          </EmptyHeader>
          {active.length > 0 ? (
            <EmptyContent>
              <Button asChild>
                <a href="/#listings">Clear filters</a>
              </Button>
            </EmptyContent>
          ) : null}
        </Empty>
        <SeedOnEmpty count={active.length} />
      </>
    );
  }

  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-7 sm:gap-x-5 sm:gap-y-9 lg:grid-cols-4">
      {listings.map((listing) => (
        <Card
          key={listing.id}
          className="group relative min-w-0 overflow-hidden transition-[box-shadow,transform] duration-200 ease-out hover:-translate-y-1 hover:shadow-[var(--shadow-border-hover)]"
        >
          <Link
            href={`/listing/${listing.slug || listing.id}`}
            seed={listing}
            className="block"
          >
            <div className="aspect-[4/5] overflow-hidden bg-muted">
              <div className="h-full w-full transition-transform duration-500 ease-out group-hover:scale-[1.025]">
                <ListingImage listing={listing} />
              </div>
            </div>
            <div className="flex flex-col gap-2.5 p-3.5 sm:p-4">
              <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                <span className="capitalize">{listing.category}</span>
                <Badge variant="outline" className="shrink-0 text-[10px]">
                  {conditionLabel(listing.condition)}
                </Badge>
              </div>
              <h2 className="line-clamp-2 min-h-10 text-sm font-medium leading-5 text-balance sm:text-[15px]">
                {listing.title}
              </h2>
              <p className="text-base font-semibold tabular-nums">
                {money(listing.price)}
              </p>
            </div>
          </Link>
          <WatchButton
            listingId={listing.id}
            listingTitle={listing.title}
            className="absolute right-3 top-3"
          />
        </Card>
      ))}
    </div>
  );
}

export default function BrowsePage({ searchParams, serverData }: PageProps) {
  const category =
    typeof searchParams.category === "string" &&
    CATEGORIES.includes(searchParams.category)
      ? searchParams.category
      : "all";
  const query = typeof searchParams.q === "string" ? searchParams.q.trim() : "";
  const sort = typeof searchParams.sort === "string" ? searchParams.sort : "latest";

  return (
    <div className="flex flex-col gap-12 pb-10">
      <section className="relative min-h-[430px] overflow-hidden rounded-[28px] bg-[#d8d4ce] shadow-[var(--shadow-border)] sm:min-h-[470px]">
        <img
          src="/images/marketplace-hero.webp"
          alt="A curated collection of secondhand furniture, clothing, audio, and camera gear"
          width="1600"
          height="1024"
          fetchPriority="high"
          decoding="async"
          className="absolute inset-0 h-full w-full object-cover object-[72%_center] outline outline-1 -outline-offset-1 outline-border sm:object-center"
        />
        <div className="absolute inset-0 bg-[#ebe8e3]/80 sm:w-[76%] sm:bg-[linear-gradient(90deg,rgba(235,232,227,.98)_0%,rgba(235,232,227,.9)_40%,rgba(235,232,227,.16)_78%,rgba(235,232,227,0)_100%)]" />
        <div className="relative flex min-h-[430px] max-w-[620px] flex-col justify-center px-6 py-12 text-[#171717] sm:min-h-[470px] sm:px-10 lg:px-12">
          <h1 className="max-w-[11ch] text-balance text-4xl font-semibold leading-[1.04] tracking-[-0.045em] sm:text-5xl lg:text-[3.5rem]">
            Better things, ready for what comes next.
          </h1>
          <p className="mt-5 max-w-[38ch] text-pretty text-base leading-7 text-[#4f4c48] sm:text-lg">
            Shop distinctive furniture, technology, fashion, and more. Make offers and follow every update live.
          </p>
          <div className="mt-7 flex flex-wrap gap-3">
            <ScrollToListingsLink
              className="inline-flex min-h-11 items-center rounded-lg bg-[#171717] px-5 text-sm font-medium text-[#fafafa] transition-[background-color,scale] duration-150 hover:bg-[#2b2b2b] active:scale-[0.96]"
            >
              Browse finds
            </ScrollToListingsLink>
            <Button
              asChild
              variant="outline"
              size="lg"
              className="bg-white/75 text-[#171717] backdrop-blur hover:bg-white hover:text-[#171717]"
            >
              <Link href="/sell">Sell</Link>
            </Button>
          </div>
        </div>
      </section>

      <Card className="grid gap-px overflow-hidden bg-border sm:grid-cols-3">
        {[
          ["Realtime catalog", "New listings and sold status update without a refresh."],
          ["Flexible offers", "Buy at the list price or send the seller an offer."],
          ["Private saves", "Keep a personal watchlist that stays synced to your account."],
        ].map(([title, body]) => (
          <div key={title} className="bg-card px-5 py-4">
            <h2 className="text-sm font-medium">{title}</h2>
            <p className="mt-1 text-pretty text-sm leading-5 text-muted-foreground">
              {body}
            </p>
          </div>
        ))}
      </Card>

      <LiveTicker />

      <section id="listings" className="flex scroll-mt-24 flex-col gap-6">
        <div>
          <h2 className="text-balance text-3xl font-semibold tracking-[-0.03em]">
            Fresh finds
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Thoughtful pieces from independent sellers.
          </p>
        </div>

        <form
          method="get"
          action="/"
          role="search"
        >
          {category !== "all" ? (
            <Input type="hidden" name="category" value={category} />
          ) : null}
          <Card className="p-3">
          <FieldGroup className="gap-3 sm:grid sm:grid-cols-[1fr_180px_auto]">
            <Field>
              <FieldLabel className="sr-only" htmlFor="catalog-search">
                Search listings
              </FieldLabel>
              <Input
                id="catalog-search"
                name="q"
                type="search"
                autoComplete="off"
                defaultValue={query}
                placeholder="Search furniture, cameras, sellers…"
                className="border-0 bg-muted shadow-none"
              />
            </Field>
            <Field>
              <FieldLabel className="sr-only" htmlFor="catalog-sort">
                Sort listings
              </FieldLabel>
              <NativeSelect
                id="catalog-sort"
                name="sort"
                defaultValue={sort}
                className="min-h-11 border-0 bg-muted shadow-none"
              >
                <NativeSelectOption value="latest">Newest first</NativeSelectOption>
                <NativeSelectOption value="price-low">
                  Price: low to high
                </NativeSelectOption>
                <NativeSelectOption value="price-high">
                  Price: high to low
                </NativeSelectOption>
              </NativeSelect>
            </Field>
            <Button type="submit" size="lg">
              Search
            </Button>
          </FieldGroup>
          </Card>
        </form>

        <nav
          aria-label="Listing categories"
          className="hide-scrollbar -mx-5 overflow-x-auto px-5 pb-1"
        >
          <div className="flex w-max gap-2">
            {CATEGORIES.map((item) => (
              <Button
                key={item}
                asChild
                variant={category === item ? "default" : "outline"}
                className="rounded-full capitalize"
              >
                <a
                  href={browseHref(item, query, sort)}
                  aria-current={category === item ? "page" : undefined}
                >
                  {item}
                </a>
              </Button>
            ))}
          </div>
        </nav>

        <Suspense
          fallback={
            <div className="grid grid-cols-2 gap-x-3 gap-y-7 sm:gap-x-5 lg:grid-cols-4">
              {Array.from({ length: 8 }).map((_, index) => (
                <Card key={index} className="overflow-hidden">
                  <Skeleton className="aspect-[4/5] rounded-none" />
                  <div className="flex flex-col gap-3 p-4">
                    <Skeleton className="h-3 w-2/5" />
                    <Skeleton className="h-9" />
                    <Skeleton className="h-4 w-1/3" />
                  </div>
                </Card>
              ))}
            </div>
          }
        >
          <Grid
            serverData={serverData}
            category={category}
            query={query}
            sort={sort}
          />
        </Suspense>
      </section>
    </div>
  );
}
